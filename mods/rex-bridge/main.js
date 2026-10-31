// Rex server groups for the existing sidebar. The external helper transports
// terminals; this view only reads connections and runs ordinary space actions.
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { hc, props, run, spaces, shared, setShared, openOverlay, theme } from "herdr";

const icon = name => `mods/rex-bridge/${name}.svg`;
const MARK = { ansi6: "#22a99a", ansi3: "#e3814f", ansi4: "#5b8def", ansi5: "#8b7cf6", ansi2: "#3fae6a", ansi1: "#d9649a" };
const palette = Object.keys(MARK);
const tint = id => palette[[...id].reduce((n, c) => n + c.codePointAt(0), 0) % palette.length];

export default class RexBridge extends View {
  init(_props, cx) {
    this.machine = props()?.machine ?? null;
    this.connections = [];
    this.loading = true;
    this.error = null;
    this.width = 240;
    this.folded = new Set(JSON.parse(localStorage.getItem("servers.folded") ?? "[]"));
    this.refresh(cx);
    // The CLI supplies connection details; spaces themselves are live Herdr data.
    cx.timer.every(15000, cx => this.refresh(cx));
  }

  refresh(cx) {
    if (this.refreshing) return;
    this.refreshing = true;
    cx.spawn(async cx => {
      try {
        const result = await hc(["machine", "ls", "--json"]);
        if (result.code !== 0) throw new Error(result.stderr || result.stdout || "Could not read machines");
        const connections = (JSON.parse(result.stdout).machines ?? [])
          .filter(m => m.via && String(m.id ?? "").startsWith("rex-") && !m.temporary);
        const names = connections.map(m => m.name).sort();
        // The paired Spaces copy uses this to omit Rex groups without losing
        // its normal machines, repo details, menus or global space indices.
        if (JSON.stringify(shared("rex-bridge.servers") ?? []) !== JSON.stringify(names)) setShared("rex-bridge.servers", names);
        this.connections = connections.filter(m => !this.machine || m.name === this.machine)
          .sort((a, b) => a.name.localeCompare(b.name));
        this.error = null;
      } catch (error) { this.error = String(error); }
      finally { this.loading = false; this.refreshing = false; cx.notify(); }
    });
  }

  session(space, t, compact) {
    const colors = shared("space.colors") ?? {};
    const color = colors[space.id] ?? tint(space.id);
    const fill = color.startsWith("#") ? color : MARK[color] ?? t[color] ?? t.accent;
    return h_flex().id(`rex-space-${space.id}`).h(30).flex_none().px(compact ? 2 : 8).gap(9).items_center().rounded(8).cursor_pointer()
      .when(space.active, el => el.bg(t.selected))
      .when(!space.active, el => el.hover(s => s.bg(t.hover)))
      .when(compact, el => el.justify_center())
      .on_click(() => run("space.select", { id: space.id }))
      .on_mouse_down("right", () => openOverlay({ mod: "menu", kind: "popup", width: 200, height: 260, props: { tab: "space", target: { id: space.id } } }))
      .child(h_flex().size(18).flex_none().rounded(5).bg(fill).items_center().justify_center().text_size(10.5).font_bold().text_color("#ffffff").child([...space.name][0]?.toUpperCase() ?? "?"))
      .when(!compact, el => el.child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(space.name))
        .child(div().text_size(11).text_color(t[space.status] ?? t.dim).child(["blocked", "done"].includes(space.status) ? "●" : "·")));
  }

  server(machine, t, compact, allSpaces, cx) {
    const connected = machine.state === "connected";
    const folded = this.folded.has(machine.name);
    const mine = allSpaces.filter(space => space.machine === machine.name);
    const head = h_flex().id(`rex-server-${machine.name}`).h(26).flex_none().px(compact ? 2 : 8).gap(7).items_center().text_size(11).text_color(t.dim)
      .cursor_pointer().hover(s => s.bg(t.hover))
      .on_click((_e, cx) => {
        folded ? this.folded.delete(machine.name) : this.folded.add(machine.name);
        localStorage.setItem("servers.folded", JSON.stringify([...this.folded]));
        cx.notify();
      })
      .child(svg(icon("server")).size(13).flex_none())
      .when(!compact, el => el.child(div().flex_1().min_w(0).truncate().child(machine.name))
        .when(!connected, el => el.child(div().flex_none().child(String(machine.state ?? "unknown").replace(/_/g, " "))))
        .when(connected, el => el.child(div().id(`rex-new-${machine.name}`).flex_none().px(4).rounded(4).hover(s => s.bg(t.hover).text_color(t.foreground))
          .on_click((_e, cx) => { cx.stop_propagation(); run("space.new", { machine: machine.name }); }).child("+"))))
      .child(div().size(6).rounded(3).flex_none().bg(connected ? t.ansi2 : t.dim))
      .child(svg(icon(folded ? "chevron-right" : "chevron-down")).size(11).flex_none());
    const rows = folded ? [] : mine.map(space => this.session(space, t, compact));
    if (!folded && !mine.length) rows.push(div().id(`rex-empty-${machine.name}`).h(28).flex_none().px(8).text_size(12).text_color(t.dim)
      .child(compact ? "…" : connected ? "No spaces yet" : "Server unavailable"));
    return [head, ...rows];
  }

  render(cx) {
    const t = theme();
    const compact = this.width < 160;
    const allSpaces = spaces();
    return v_flex().id("rex-sidebar").size_full().min_h(0).bg(t.chrome).text_size(13)
      .on_size_change((size, cx) => { if (this.width !== size.width) { this.width = size.width; cx.notify(); } })
      .child(h_flex().h(31).flex_none().pl(compact ? 6 : 16).pr(10).items_center().justify_between().text_size(11.5).text_color(t.dim)
        .child(compact ? "R" : "Rex")
        .child(div().id("rex-refresh").size(26).flex_none().flex().items_center().justify_center().rounded(6).cursor_pointer().hover(s => s.bg(t.hover).text_color(t.foreground))
          .on_click((_e, cx) => this.refresh(cx)).child("↻")))
      .child(v_flex().flex_1().min_h(0).px(8).pb(10).overflow_y_scroll()
        .when(Boolean(this.error), el => el.child(div().p(8).text_size(11).text_color(t.muted).child(this.error)))
        .when(this.loading && !this.connections.length, el => el.child(div().px(8).py(6).text_size(12).text_color(t.dim).child(compact ? "…" : "Loading…")))
        .when(!this.loading && !this.connections.length && !this.error, el => el.child(div().p(8).text_size(12).text_color(t.dim).child(compact ? "—" : "Ask your agent to connect a Rex server.")))
        .children(this.connections.flatMap(machine => this.server(machine, t, compact, allSpaces, cx))));
  }
}
