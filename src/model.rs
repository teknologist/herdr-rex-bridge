pub fn pane(id: &str, address: &str) -> crate::layout::Node {
    crate::layout::Node::block(
        id,
        crate::layout::Content::Terminal {
            terminal: address.into(),
        },
    )
}
