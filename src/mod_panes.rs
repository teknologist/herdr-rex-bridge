//! Herdr-only mod panes, attached to stable Rex terminal addresses.
//! Rex keeps the terminal layout. These decorations live solely in bridge metadata.
use crate::layout::{Axis, Content, Kind, Node, Size};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decoration {
    anchor: String,
    before: bool,
    column: bool,
    node: Node,
    anchor_size: Option<Size>,
}
pub type Decorations = BTreeMap<String, Vec<Decoration>>;

fn weight(node: &Node) -> f32 {
    match node.size {
        Some(Size::Weight { weight }) => weight,
        _ => 1.0,
    }
}
fn terminal(node: &Node, last: bool) -> Option<String> {
    let nodes = node.terminals();
    (if last { nodes.last() } else { nodes.first() })
        .and_then(|n| n.terminal())
        .map(str::to_owned)
}
fn has_mod(node: &Node) -> bool {
    node.walk()
        .iter()
        .any(|n| matches!(n.content(), Some(Content::Mod { .. })))
}

/// Remove mod branches before planning Rex calls, restoring their share to
/// the neighboring terminal. Adding an Easel must not resize Rex's own splits.
fn strip(node: &Node, out: &mut Vec<Decoration>) -> Option<Node> {
    if let Kind::Block { .. } = node.kind {
        return node.terminal().is_some().then(|| node.clone());
    }
    let children = node.children();
    let live: Vec<usize> = children
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.terminals().is_empty())
        .map(|(i, _)| i)
        .collect();
    if live.is_empty() {
        return None;
    }
    let mut additions = BTreeMap::<usize, f32>::new();
    for (i, child) in children.iter().enumerate() {
        if !child.terminals().is_empty() || !has_mod(child) {
            continue;
        }
        let neighbor = live
            .iter()
            .copied()
            .rev()
            .find(|j| *j < i)
            .unwrap_or(live[0]);
        let before = i < neighbor;
        if let Some(anchor) = terminal(&children[neighbor], !before) {
            out.push(Decoration {
                anchor,
                before,
                column: matches!(node.kind, Kind::Column { .. }),
                node: child.clone(),
                anchor_size: children[neighbor].size,
            });
            *additions.entry(neighbor).or_default() += weight(child);
        }
    }
    let mut kept = Vec::new();
    for i in live {
        let mut child = strip(&children[i], out)?;
        if let Some(extra) = additions.get(&i) {
            child.size = Some(Size::Weight {
                weight: weight(&child) + extra,
            });
        }
        kept.push(child);
    }
    let mut result = node.clone();
    *result.children_mut()? = kept;
    Some(result)
}

/// Returns a terminal-only document and its separately persisted mod panes.
/// Mod-only Rex tabs have no real Rex window to attach to; reject them before
/// changing the server rather than manufacturing a placeholder terminal.
pub fn capture(document: &Value) -> anyhow::Result<(Value, Decorations)> {
    let mut clean = document.clone();
    let mut decorations = Decorations::new();
    for space in clean["spaces"].as_array_mut().into_iter().flatten() {
        for tab in space["tabs"].as_array_mut().into_iter().flatten() {
            let root: Node = serde_json::from_value(tab["root"].clone())?;
            let mut mods = Vec::new();
            if has_mod(&root) && root.terminals().is_empty() {
                anyhow::bail!(
                    "a mod in a Rex tab needs a terminal beside it; use a local mod tab for a standalone canvas"
                );
            }
            if let Some(root) = strip(&root, &mut mods) {
                tab["root"] = serde_json::to_value(root)?;
            }
            if !mods.is_empty() {
                decorations.insert(tab["id"].as_str().unwrap_or_default().to_owned(), mods);
            }
        }
    }
    Ok((clean, decorations))
}

fn attach(node: &mut Node, decoration: &Decoration) -> bool {
    if node.terminal() == Some(decoration.anchor.as_str()) {
        let mut anchor = node.clone();
        let outer_size = anchor.size;
        anchor.size = decoration.anchor_size;
        let children = if decoration.before {
            vec![decoration.node.clone(), anchor]
        } else {
            vec![anchor, decoration.node.clone()]
        };
        let mut wrapper = Node::container(
            format!("mod-wrap-{}", decoration.node.id),
            if decoration.column {
                Axis::Column
            } else {
                Axis::Row
            },
            children,
        );
        wrapper.size = outer_size;
        *node = wrapper;
        return true;
    }
    if let Some(children) = node.children_mut() {
        for child in children {
            if attach(child, decoration) {
                return true;
            }
        }
    }
    false
}

/// Reapply decorations to a fresh authoritative Rex projection. Missing
/// anchors are omitted; new Rex terminals and reordered splits remain intact.
pub fn apply(document: &mut Value, decorations: &Decorations) {
    for space in document["spaces"].as_array_mut().into_iter().flatten() {
        for tab in space["tabs"].as_array_mut().into_iter().flatten() {
            let Some(mods) = tab["id"].as_str().and_then(|id| decorations.get(id)) else {
                continue;
            };
            let Ok(mut root) = serde_json::from_value::<Node>(tab["root"].clone()) else {
                continue;
            };
            // After-anchor siblings are nested in reverse so their visual order
            // remains the original order. Before-anchor siblings retain order.
            for d in mods
                .iter()
                .rev()
                .filter(|d| !d.before)
                .chain(mods.iter().filter(|d| d.before))
            {
                attach(&mut root, d);
            }
            if let Ok(value) = serde_json::to_value(root) {
                tab["root"] = value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn terminal_node(id: &str) -> Node {
        Node::block(
            format!("pane-{id}"),
            Content::Terminal {
                terminal: id.into(),
            },
        )
    }
    fn doc(root: Node) -> Value {
        json!({"spaces":[{"id":"s1","tabs":[{"id":"w1","root":root}]}]})
    }
    fn root(doc: &Value) -> Node {
        serde_json::from_value(doc["spaces"][0]["tabs"][0]["root"].clone()).unwrap()
    }
    fn example() -> Value {
        let mut t = terminal_node("b1");
        t.size = Some(Size::Weight { weight: 0.25 });
        let mut m = Node::block(
            "easel",
            Content::Mod {
                name: "easel".into(),
                props: json!({"project":"/worktree"}),
            },
        );
        m.size = Some(Size::Weight { weight: 0.25 });
        let mut t2 = terminal_node("b2");
        t2.size = Some(Size::Weight { weight: 0.5 });
        doc(Node::container("w1", Axis::Row, vec![t, m, t2]))
    }
    #[test]
    fn mod_share_does_not_resize_rex_terminals_and_props_survive_reprojection() {
        let (mut clean, mods) = capture(&example()).unwrap();
        assert_eq!(root(&clean).children().len(), 2);
        assert_eq!(
            root(&clean).children()[0].size,
            Some(Size::Weight { weight: 0.5 })
        );
        apply(&mut clean, &mods);
        let r = root(&clean);
        assert_eq!(r.terminals().len(), 2);
        assert!(
            matches!(r.find("easel").unwrap().content(),Some(Content::Mod{props,..}) if props["project"]=="/worktree")
        );
    }
    #[test]
    fn new_rex_splits_and_saved_metadata_keep_the_mod_without_fake_terminals() {
        let (_, mods) = capture(&example()).unwrap();
        let mods: Decorations =
            serde_json::from_str(&serde_json::to_string(&mods).unwrap()).unwrap();
        let mut changed = doc(Node::container(
            "w1",
            Axis::Column,
            vec![
                terminal_node("b2"),
                terminal_node("b1"),
                terminal_node("b3"),
            ],
        ));
        apply(&mut changed, &mods);
        let r = root(&changed);
        assert_eq!(r.terminals().len(), 3);
        assert!(r.find("easel").is_some());
        let (clean, again) = capture(&changed).unwrap();
        assert!(!has_mod(&root(&clean)));
        assert_eq!(again["w1"].len(), 1);
    }
    #[test]
    fn removing_the_mod_clears_metadata_and_lost_anchors_do_not_resurrect_terminals() {
        let (_, mods) = capture(&example()).unwrap();
        let mut changed = doc(Node::container("w1", Axis::Row, vec![terminal_node("b2")]));
        apply(&mut changed, &mods);
        assert!(!has_mod(&root(&changed)));
        assert!(capture(&changed).unwrap().1.is_empty());
    }
    #[test]
    fn mod_only_tabs_fail_before_any_rex_mutation() {
        let m = Node::block(
            "m",
            Content::Mod {
                name: "easel".into(),
                props: Value::Null,
            },
        );
        assert!(capture(&doc(m)).is_err());
    }
}
