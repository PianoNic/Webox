# Theming and layout engine (design)

Status: **design, phase 1**. No code changes yet. Implementation starts after the English-strings sweep across `src/ui`.

## Summary

Webox draws its UI in immediate-mode egui, with the look spread across the code: about 160 `t.tk("…")` token lookups,
about 250 literal `hex(0x…)` colours and about 380 `FontId::proportional(…)` / `semibold(…)` sizes in `src/ui`. The window
arrangement is a fixed sequence of egui panels in `src/ui/frame.rs`. Plugins can recolour that UI (palette 1.0, accents 1.3,
decorations 1.4) and hide or reorder a few buttons ("Oberfläche anpassen", `src/ui/customize.rs`). They cannot change
geometry, fonts or arrangement.

This design turns the UI into an engine with three layers that plugins drive with **data**, never with drawing code:

1. **Theme**: one resolved theme supplies every colour, font, size, radius, spacing, stroke, shadow, motion timing and icon.
   The Momentum tokens stay the default theme.
2. **Layout**: the window is a declarative tree of regions (docks, splits, stacks, switches, overlays, floating panes) whose
   leaves are **components**. The current Webex arrangement is the built-in tree. A plugin can ship a different tree: chat
   list on the right, a single column, a Discord-style server bar, or two panes with no rail.
3. **Components** keep their behaviour inside Webox and publish a typed **contract**: inputs, actions, slots, variants,
   parameters and size limits. Plugins place and style components and fill their slots with a small, safe widget
   description. They never draw egui across the C ABI.

Everything is resolved **once per change** (theme switch, plugin toggle, file edit, window-size class change) into flat,
index-addressed structures. A frame reads fields and indices. It does no JSON parsing, no string lookups on hot paths and
no new allocations. This also removes three per-frame C-ABI calls that happen today (see "Where we are today").

The migration runs in ten phases, roughly 65–85 agent-days in total. Every phase is gated on pixel-identical screenshots of
the default look.

---

## Behaviour and coherence

This section is the backbone of the design. A plugin may rearrange and restyle anything, but **behaviour stays correct
and consistent**. Everything else in this document serves that rule.

### 1. Components own behaviour; plugins own placement and look

A component is Rust code in Webox that owns all of its behaviour:

- selection, unread and read state, and read receipts;
- scrolling, jump-to-message, the "new messages" banner and the autoscroll anchor;
- drag and drop (files onto the composer or the conversation, reordering), context menus, tooltips;
- keyboard shortcuts, focus, text selection, IME and accessibility labels;
- animations (respecting `fx::reduce_motion()`), and the loading, empty and error states.

A layout can only **place** a component (in a region, with size constraints), **style** it (tokens, declared variants,
clamped parameters) and **fill its declared slots**. It has no path to a component's internals, so it cannot break them. A
message list in the right pane, or in a floating window, still marks messages read, still keeps its scroll anchor and
still opens the same context menu.

### 2. Component contracts are the public UI API

Every component declares a contract. These contracts are the plugin-facing UI API: typed, versioned (`message_list@1`)
and documented in `docs/plugins.md` style, with one section per component.

```rust
/// Static, compiled into Webox. One per component.
pub(crate) struct ComponentSpec {
    pub id: ComponentId,              // e.g. ComponentId::MessageList, string "message_list"
    pub version: u16,                 // contract version; additions keep it, incompatible changes get a new id
    pub need: Need,                   // Required / RequiredReachable / Optional
    pub size: SizeHints,              // min / preferred / max, can_collapse
    pub reads: &'static [StateKey],   // inputs: the app state it shows ("space.selected", "space.messages", ...)
    pub emits: &'static [ActionId],   // outputs: actions it dispatches ("space.open", "message.send", ...)
    pub slots: &'static [SlotSpec],   // where extra content may go ("header.actions", "footer", "row.trailing")
    pub variants: &'static [VariantSpec], // enumerated looks ("style": bubbles | flat | compact)
    pub params: &'static [ParamSpec], // numeric / bool knobs with ranges ("avatar_size": 16..=48)
    pub parts: &'static [&'static str],   // stylable parts for component tokens ("row", "row.avatar", "bubble")
}
pub(crate) enum Need { Required, RequiredReachable, Optional }
pub(crate) struct SizeHints { pub min: Vec2, pub pref: Vec2, pub max: Vec2, pub can_collapse: bool }
pub(crate) struct SlotSpec { pub id: &'static str, pub max_items: u8, pub accepts: SlotKind }
pub(crate) enum SlotKind { Actions, Widgets, Badges }
pub(crate) struct VariantSpec { pub id: &'static str, pub options: &'static [&'static str], pub default: &'static str }
pub(crate) struct ParamSpec { pub id: &'static str, pub kind: ParamKind, pub default: f32 }
pub(crate) enum ParamKind { Range(f32, f32), Bool, Token /* a token reference */ }
```

A layout file refers to components and their slots, variants and params by these string ids. Unknown ids fail validation
with a message naming the contract version Webox has.

### 3. Shared state and one action bus

Components never call each other. They read one **app state** (today the `App` struct in `src/main.rs`) through named
**state keys**, and they change it only through **actions** dispatched on one bus. The list and the chat therefore stay in
sync wherever they are placed. A plugin button triggers exactly the same action, with the same rules.

The bus grows from what exists. `enum Act` in `src/main.rs` (`Tab`, `Prev`, `Next`, `Back`, `Favorite`, `ToggleRail`,
`Composer(&str)`, ...) is already dispatched by `App::act` (`src/app/mod.rs`) and named by the shortcut registry ids in
`src/shortcuts/`. It becomes:

```rust
/// Stable string ids ("space.open", "composer.focus", "call.start", "panel.toggle", "nav.back").
pub(crate) enum Action {
    NavPage(Page), NavBack, NavForward,
    SpaceOpen(SpaceRef), SpaceFavorite(SpaceRef, bool), SpaceLeave(SpaceRef),
    ComposerFocus, ComposerInsert(String), ComposerCommand(&'static str),
    CallStart { peer: PeerRef, video: bool }, CallJoinSpace(SpaceRef), CallHangUp, CallToggleMute,
    PanelToggle(PanelId), PanelSet(PanelId, bool), FocusMode(bool),
    SearchOpen, SettingsOpen(Option<&'static str>),
    PluginAction { plugin: PluginKey, view: ViewId, id: String },
    // ... one per existing Act and shortcut command
}
impl App {
    /// The only way UI changes app state. Checks the same guards for every caller: a read-only or closed space
    /// (space.closed), call state (`locus::call_allowed`), the Settings reachability rule from customize.rs, permissions.
    pub(crate) fn dispatch(&mut self, a: Action, origin: Origin) -> Result<(), Refusal>;
}
```

`Origin` records the source (shortcut, component, plugin view) for logs and for the rule "plugins may not trigger X". A
refusal shows the same toast as the built-in path. The call-button code in `conversation.rs` already routes through
`start_call_with` / `locus::call_allowed`; the bus makes that the general rule.

State keys are read-only, typed and cheap: `nav.page`, `space.selected`, `space.selected.direct`, `space.selected.closed`,
`call.active`, `call.state`, `window.width_class`, `ui.focus_mode`, `ui.rail_wide`, `panel.<id>.open`, `unread.count`.
Layout conditions and plugin widget bindings use only these keys.

### 4. Global interaction rules stay global

The following are not components and cannot be placed or removed. Every layout gets them automatically:

- **Shortcuts**: the registry in `src/shortcuts/`. Bindings dispatch actions, never component internals.
- **Focus traversal**: egui moves focus in widget creation order. The engine draws regions in tree order, so Tab order
  follows the layout tree. A component may mark itself `focus_skip` (decorative panes).
- **Escape and menu stacking**: `src/ui/menu/` keeps the one open-menu stack. Escape closes the topmost menu, then dialog,
  then overlay, regardless of layout.
- **Drag and drop**: drop targets are declared by components (composer, conversation). The engine routes OS file drops to
  the component under the pointer, and to the selected space's composer when nothing claims them.
- **Window chrome and hit-testing**: the frameless window's resize edges (`chrome::edge_strips`), caption drag area and
  window controls. If a layout omits the header, the engine adds a minimal caption strip with the controls.
- **Notifications, toasts, tray, taskbar badge** (`notify_update`), and the DPI, zoom, density and light/dark/high-contrast
  switching.

### 5. Validation and fallback

A layout must place every `Required` component, and give every `RequiredReachable` component a route (a toggle action,
a switch case or a menu entry; for example "settings reachable via the rail gear or the header avatar menu", which
generalises today's rule in `customize::can_hide`). It must also satisfy the size constraints at the smallest supported
window. Invalid input never half-applies: Webox falls back to the default layout and theme, keeps the plugin enabled,
and shows an error that names the file, the JSON path and the reason. Plugin authors run the same checks with
`webox --lint-ui <dir>` (section 5).

### 6. Consistency through tokens and primitives

Tokens and components are the only way to style. Plugin content (slots, panels) is built from shared primitives (button,
list row, menu, dialog, input, badge) that read the same resolved theme as the built-in UI. A new layout therefore still
feels like one app: one radius scale, one type scale, one focus ring, one hover language.

### 7. One behaviour suite, several layouts

The behaviour tests run against the default layout **and** the example alternative layout (example B below). They cover
keyboard navigation, selection sync between list and chat, menus, composer, calls and focus order. A layout-specific
failure is a bug in the engine or a contract, never "expected for that layout" (see "Testing").

---

## Where we are today

What the engine builds on, with the costs it removes.

**Theme tokens.** `src/ui/theme/tokens.rs` (generated) holds `TOKENS: &[(&str, [u32; 13])]`, sorted by name, one column
per `ThemeId` (ten pickable Momentum themes, Rose, and high contrast). `theme::tok(theme, name)` binary-searches it;
high contrast maps tokens to Windows system colours through `SYS` / `hc::sys_colour`. `Th` (macro `th!` in
`src/ui/theme/mod.rs`) is a `Copy` struct of about 40 semantic colour fields (`win`, `card`, `text1`, `accent`, ...), each
bound to one token. `Th::tk(name)` resolves any token: first a linear scan of the accent `OVERRIDES` thread-local, then
the binary search.

**Density.** `src/ui/theme/density.rs`: `Metrics` (about 45 `f32` spacings), two consts `STANDARD` / `COMPACT` and an
atomic, read through `density::m()`. `set_from(&settings)` reads the JSON every frame. The tests guarantee that compact
deltas stay on the 4 pt grid.

**Applying the theme.** `App::apply_theme` (`src/app/mod.rs`) runs **every frame**. It builds `Th::of(id)`, calls the
plugins' `palette()`, `decor(&info)` and `accents(&info)` across the C ABI (`Plugins::palette/decor/accents` in
`src/extras/plugins/mod.rs`), converts the decorations (`decor::from_api`, which allocates `Vec`s), and replaces the
override `Vec`s (`set_accent_overrides`). It calls `ctx.set_visuals` only when `Th` changed. Readers such as
`accent_gradient()` and `decor::stripes()` clone `Vec`s on each call.

**Fonts.** `set_fonts` in `src/main.rs`: Segoe UI and Segoe UI Semibold from `%WINDIR%\Fonts` (a `semibold` family), the
emoji font, and Windows fallbacks. Selawik is bundled for Linux. Sizes are literals at each call site.

**Icons.** `src/assets/icons.rs`: `Icons::paint(ui, name, rect, tint)` rasterises Momentum SVGs by name with a cache.
`set_art_ink` tints line art.

**Window arrangement.** `src/ui/frame.rs` `update()` paints the window gradient and the decor wash, then calls in order
`edge_strips`, `top_bar` (`TopBottomPanel "top"`), `connectivity_banner`, `rail` (`SidePanel "rail"`,
`src/ui/chrome/rail.rs`) and `decor::chrome`. It then switches on `self.tab` between `messaging` (`SidePanel "spaces"` +
`list_splitter` + `CentralPanel` with `conversation`, `src/ui/messaging.rs`), `settings_tab` and `pages_frame`
(`src/ui/page_kit.rs`). Popups, the call overlay, toasts and effects follow. The conversation (`src/ui/conversation.rs`)
paints the banner, favourite star, action buttons, tabs, message list and composer at density-driven offsets inside one
rect. Call UI lives in separate immediate viewports (`src/ui/call_win/`: call window, dock, plus `call_toast.rs`).

**Customisation.** `src/ui/customize.rs` defines seven `Group`s (rail, more menu, composer toolbar, header, space tabs,
message bar, list) with stored `hidden` / `order`. The layout code iterates `customize::visible(&settings, &G)` each
frame. That walks the settings JSON but allocates nothing.

**Plugins.** Native `cdylib`s behind `PluginVTable` / `HostApi` (`plugin-api/src/lib.rs`, contract 1.6, `ABI_VERSION`
1). Since 1.6, capabilities are named host methods and events with JSON payloads (`METHODS` in
`src/extras/plugins/mod.rs`). Some host-rendered declarative UI exists already: plugin settings pages (schema JSON),
menu entries (`message.menu` / `space.menu`, `MenuItem`) and `ui.dialog` (tabs of rich-text spans). Plugins cannot draw:
`src/extras/gif_extras/mod.rs` is in-tree "because dynamic plugins cannot draw egui UI safely across the C ABI".

**Parity tooling.** `--sample --screenshot <png>` with `WEBOX_SHOT_*` variables (theme, density, zoom, tab, menu, call
state), driven by `scripts/shots.ps1` and `scripts/pride-shots.ps1`. Scripted input comes from `src/ui/shot_input.rs`.

---

## 1. Theme layer

### Model

A theme is a set of **values** addressed by **keys** in three tiers. Each tier may refer to the one below.

1. **Core tokens**: the Momentum names exactly as in `tokens.rs` (`text-primary`, `button-primary-background:hovered`,
   `theme-gradient-secondary-0-background`, ...). The default themes stay generated from the Cisco token tables.
2. **Roles**: today's `Th` fields under stable names (`role.win`, `role.card`, `role.text1`, `role.accent`, ...), bound
   to core tokens as in `th!`.
3. **Component tokens**: `<component>.<part>.<property>[:state]`, for example `space_list.row.background:hovered`,
   `message_list.bubble.radius` or `composer.focus_line.gradient`. Their defaults point at roles or core tokens, so
   the default theme looks exactly as today.

Value kinds:

```rust
pub(crate) enum Value {
    Color(Color32),            // "#RRGGBBAA", or a reference "{text-primary}", or "{role.accent} @ 40%"
    Gradient(GradId),          // stops + angle, stored once in ResolvedTheme::gradients
    Length(f32),               // pt
    Radius([f32; 4]),
    Stroke(f32, Color32),
    Shadow(egui::epaint::Shadow),
    Font(FontRef),             // family + size + weight role
    Motion { secs: f32, ease: Ease },
    Icon(IconRef),             // set + name
    Flag(bool),
}
```

### Resolution: once per change

```rust
/// Built when something it depends on changes; shared as Arc, never mutated.
pub(crate) struct ResolvedTheme {
    pub gen: u64,                          // bumped on every rebuild
    pub id: ThemeId, pub light: bool, pub hc: bool,
    pub th: Th,                            // the familiar struct, so existing code keeps compiling
    pub colors: Box<[Color32]>,            // index = TokenId (core + component tokens)
    pub lengths: Box<[f32]>,               // index = LenId
    pub radii: Box<[[f32; 4]]>, pub strokes: Box<[Stroke]>, pub shadows: Box<[Shadow]>,
    pub fonts: Box<[FontId]>,              // index = FontRole (Body, Secondary, Title, Banner, Mono, ...)
    pub motion: Box<[Motion]>,             // index = MotionId (RailExpand = 0.18 s, ...)
    pub gradients: Box<[Gradient]>,        // accent gradient, decor washes
    pub metrics: Metrics,                  // density table after overrides
    pub icons: IconSetId,
    pub decor: Option<DecorResolved>,      // today's decor::D, without per-call clones
    pub visuals: egui::Visuals,            // computed here, set on ctx only when gen changes
}
```

- **Ids are compile-time constants.** `tokens.rs` is generated and sorted, so the generator also emits
  `pub const TEXT_PRIMARY: TokenId = TokenId(1234);` and a `tok!("text-primary")` macro that expands to that const. A
  typo becomes a compile error. Hot paths read `rt.colors[id]` as one array index. `Th::tk(&str)` stays as a compatibility
  shim (binary search) for migration and for names that come from plugin data.
- **Rebuild triggers**: theme or density setting changed (from `settings.changed`, not a per-frame JSON read), a plugin
  enabled or disabled, a UI package reloaded, the OS high-contrast or dark state changed (`hc_cached` already polls every
  2 s), or a design plugin announcing a change through a new `theme.invalidate` method. Legacy `palette` / `accents` /
  `decor` hooks are called **during a rebuild only**, not every frame.
- **Per frame**: `apply_theme` becomes "if `ctx` holds an older `gen`, set fonts and visuals; return `&rt`". No ABI calls,
  no allocation.

### Light, dark, high contrast

A theme file declares `extends` (a Momentum theme id or another plugin theme) and per-mode blocks:

```json
{ "extends": "MomentumDark", "modes": { "dark": { ... }, "light": { ... } }, "high_contrast": "inherit" }
```

The active mode follows the Webex rule (the picker's theme id decides dark or light; `dark_variant()` is used for calls
with "always dark theme for calls"). `high_contrast: "inherit"` (the default) means that while Windows high contrast is
on, a plugin theme's colours are ignored and only its geometry and fonts apply. This matches today's behaviour, where the
picker is disabled under high contrast. A theme may opt in with its own `high_contrast` block, which must pass the
contrast lint.

### Density

`Metrics` moves into `ResolvedTheme::metrics`. A theme may override fields by name for either density:

```json
"density": { "standard": { "row_h": 56, "row_pitch": 57 }, "compact": { "row_h": 44, "row_pitch": 45 } }
```

The existing invariants (`compact_rows_fit_their_content`, the 4 pt grid) run as lint rules on the resolved values. A
violation is a warning for plugin themes and stays a test failure for the built-in tables. The list's compact view
(`ListRow::COMPACT_VIEW`) becomes the `space_list` variant `row: one_line`.

### Fonts

```json
"fonts": {
  "files": { "inter": ["fonts/Inter-Regular.ttf"], "inter-semibold": ["fonts/Inter-SemiBold.ttf"] },
  "families": { "body": ["inter", "@system"], "semibold": ["inter-semibold", "@system-semibold"] },
  "scale": { "body": 14.5, "secondary": 13, "title": 15.6, "banner": 20, "caption": 12 }
}
```

- `@system` / `@system-semibold` mean today's fonts (Segoe UI, or Selawik on Linux) with the emoji and fallback chain of
  `set_fonts`. Plugin fonts are **prepended**, so missing glyphs still fall back.
- Font bytes load on a worker thread when the package loads. `ctx.set_fonts` runs only on a rebuild that changed the font
  set. Limits: TTF/OTF only, 8 MB per file, 24 MB per package. The 14 MB of system fonts are shared, never reloaded.
- Call sites move from `FontId::proportional(13.0)` to `rt.font(FontRole::Secondary)`. The scale defaults are today's
  literals, so default output stays identical.

### Icons

An icon set maps Momentum icon names (`chat-filled`, `handset-regular`, ...) to SVG files in the package. Names it does
not cover fall back to Momentum. `Icons::paint` keys its raster cache on `(IconSetId, name, size_px)`. A set switch
clears the cache once. SVGs are parsed with the existing renderer, capped at 64 KB each, with external references
stripped.

### Shadows, strokes, radii, motion

These become tokens with today's literals as defaults. Examples: card radius `10.0` (`messaging.rs`, `conversation.rs`),
pill radius, the rail animation `0.18 s` (`chrome/rail.rs`, `messaging.rs`), the call control bar's `BAR_OPACITY` and
auto-hide delay (`call_win/mod.rs`, kept as theme values only where they are look, not behaviour). Motion values are
multiplied by 0 while `fx::reduce_motion()` is true.

---

## 2. Layout layer

### The tree

```rust
pub(crate) enum Node {
    /// Attach `child` to one side of the remaining space (the egui SidePanel / TopBottomPanel model).
    Dock { id: NodeId, side: Side, size: Size, child: Box<Node> },
    /// Split the remaining space along `axis`. Ratios are persisted per node id (today's "list_w").
    Split { id: NodeId, axis: Axis, items: Vec<SplitItem>, splitter: Option<SplitterStyle> },
    /// Children one after another, each at its preferred size (toolbars, a banner above a list).
    Stack { id: NodeId, axis: Axis, gap: LenRef, children: Vec<Node> },
    /// One child chosen by a state key (pages, call vs. no call, narrow vs. wide).
    Switch { id: NodeId, on: StateKey, cases: Vec<(Match, Node)>, default: Box<Node> },
    /// Drawn above its parent's area, anchored (banners, the jump-to-latest button, splash).
    Overlay { id: NodeId, anchor: Align2, offset: Vec2, child: Box<Node> },
    /// A movable, resizable pane inside the window (not an OS window); rect persisted per id.
    Floating { id: NodeId, default: RectSpec, size: Size, child: Box<Node> },
    /// Rendered only while `cond` holds.
    When { cond: Cond, child: Box<Node> },
    /// A surface card (background gradient, radius, wash) around a child: the list card, the content card.
    Card { id: NodeId, style: CardStyle, child: Box<Node> },
    /// A leaf: a component with its variants, params and slot contents.
    Slot(Placement),
}
pub(crate) struct SplitItem { pub node: Node, pub size: Size }
pub(crate) struct Size { pub min: f32, pub pref: f32, pub max: f32, pub ratio: Option<f32>,
                         pub resizable: bool, pub collapse_below: Option<f32> }
pub(crate) enum Cond { WidthBelow(f32), WidthAtLeast(f32), Key(StateKey, Match), Not(Box<Cond>), All(Vec<Cond>), Any(Vec<Cond>) }
pub(crate) struct Placement { pub component: ComponentId, pub variants: VariantSet, pub params: ParamSet,
                              pub slots: Vec<(SlotId, Vec<SlotItem>)>, pub style: Vec<(TokenId, Value)> }
```

**Surfaces.** The main window, the call window and the calling dock each have their own tree. The incoming-call toast
and the OS notification windows are not layoutable (see "What stays hard-coded").

### The default layout as data

The built-in instance, written in the same format a plugin uses. It is compiled into Webox, and the engine's tests check
that it validates.

```json
{
  "schema": "webox-ui-layout/1",
  "id": "webex-default",
  "surfaces": { "main": {
    "dock": "top", "id": "header", "size": { "pref": "{metrics.header_h}" },
    "child": { "slot": "header" },
    "rest": {
      "dock": "left", "id": "rail", "size": { "pref": 60.8, "wide": 200 },
      "child": { "slot": "rail", "variants": { "orientation": "vertical", "labels": "when_wide" } },
      "rest": { "switch": "nav.page", "cases": {
        "messaging": { "split": "row", "id": "msg-split", "splitter": "gap4", "items": [
          { "size": { "min": 280, "pref": 367, "max": 400, "resizable": true },
            "when": { "not": { "key": "ui.list_hidden" } },
            "node": { "card": "list", "child": { "slot": "space_list" } } },
          { "size": { "min": 560 }, "node": { "card": "content", "child": { "slot": "conversation" } } } ] },
        "settings": { "split": "row", "items": [
          { "size": { "pref": 240 }, "node": { "slot": "settings_nav" } },
          { "node": { "slot": "settings_page" } } ] },
        "*": { "slot": "page" } } } } } }
}
```

`conversation` is a **composite component**: by default it expands to
`Stack[chat_header, space_tabs, Switch(conv.tab){messages: message_list, ...}, composer]`. A layout may use it whole (most
themes) or place its parts separately (example B puts the composer in its own dock). The composite keeps the
cross-part behaviour, such as the composer's height pushing the list's bottom and reply/edit banners, through state, not
geometry.

`"rest"` is shorthand: the remaining space after a `dock` goes to the next node. The `Node` enum above is the parsed
form.

### Computing rects without per-frame cost

- **Compile once.** A validated tree compiles to `CompiledLayout { nodes: Box<[CNode]>, conds: Box<[CCond]> }`. Nodes
  are in draw order and refer to each other by index. State keys become `StateKeyId(u16)`. Conditions become bit tests.
- **Solve on change.** Per frame, the engine builds a small `StateBits` (u64: page, call active, focus mode, panels open,
  width class) from `App` fields. That is a handful of comparisons, no strings. Rects are re-solved only when
  `(screen_rect, StateBits, splitter values, metrics gen)` differs from last frame. Otherwise the cached
  `Box<[Rect]>` is reused. The solve itself is a linear pass with a reused scratch buffer.
- **Render.** For each visible leaf in order, the engine calls the component's `show(app, ui, rect, &placement, &rt)` in
  a child `Ui` (`ui.new_child(UiBuilder::new().max_rect(r))`). Floating and overlay nodes use `egui::Area`. egui panels
  disappear from the main surface, so `decor::chrome`'s `PanelState::load("rail")` becomes a lookup in the solved rects.
- **Width classes** (`narrow < 720 ≤ medium < 1100 ≤ wide`, theme-adjustable) are bits, so a responsive switch costs
  nothing until the class changes.
- **Collapse.** When the window is smaller than the sum of the mins, nodes with `collapse_below` collapse in reverse tree
  order. A collapsed `RequiredReachable` component must have a toggle in the tree. Lint checks this.

### Visibility rules

`When` and `Switch` read only state keys: `window.width_class`, `call.active`, `ui.focus_mode`, `nav.page`,
`space.selected`, `panel.<id>.open`, `ui.rail_wide`, `ui.list_hidden`. Panels a layout invents (`panel.members`) are
declared in the layout, toggled by `PanelToggle` actions and persisted like `list_w` today.

### "Oberfläche anpassen" becomes a patch layer

The seven groups of `customize.rs` become **slot item lists** of their components: `rail.items`, `rail.more_menu`,
`composer.toolbar`, `header.actions`, `space_tabs.tabs`, `message_list.hover_bar` and `space_list.top`. The stored
`ui_*` keys stay, so existing settings files keep working. They are read into a **user patch** applied on top of
whichever layout is active, at resolve time and not per frame (today `customize::visible` walks the JSON every frame). The
Settings page keeps its look. It edits the patch and lists the slots of the active layout. `can_hide`'s rule (Settings
stays reachable) becomes a general `RequiredReachable` check.

---

## 3. Component styling and variants

Restyling without code works on three levels, from cheapest to richest.

1. **Tokens**: any component token (`space_list.row.height`, `message_list.bubble.background.own`,
   `rail.item.radius`), set in a theme or on a single placement (`"style": { ... }`).
2. **Variants**: enumerated alternatives implemented in Rust, chosen by name. Each variant is a code path Webox tests.
3. **Params**: numeric or boolean knobs, clamped to the contract's range.

The initial catalogue, all implemented in Webox:

- `message_list`: `style` = `flat` (today) | `bubbles` | `compact`; `own_align` = `left` | `right` (bubbles only);
  `avatar` = `circle` | `rounded` | `square` | `none`; `group_window_s` (the merge window for consecutive messages);
  `timestamp` = `header` | `hover` | `trailing`; `hover_bar` = `top_right` | `inline`.
- `space_list`: `row` = `two_line` | `one_line` (today's compact view) | `card`; `avatar` shape and size; `sections` =
  `favorites_first` | `flat`; `preview` = on | off; `unread` = `dot` | `bold` | `badge`.
- `rail`: `orientation` = `vertical` | `horizontal`; `labels` = `never` | `when_wide` | `always`; `item` = `circle` |
  `pill` | `square`; `source` = `pages` (today) | `teams_and_spaces` (Discord-style server bar: favourite spaces and teams
  as avatars, `nav.page` entries collapse into a top button).
- `chat_header`: `size` = `banner` (116 pt cover banner) | `bar` (44 pt, no cover); `actions` = `round` | `text` | `icons`.
- `composer`: `toolbar` = `below` (today) | `above` | `inline` | `hidden`; `send` = `icon` | `text` | `none`; `shape` =
  `flat` | `pill` | `card`.
- Shared primitives: `button.style` = `filled` | `pill` | `ghost` | `outline`; `menu.style` = `webex` | `dense`;
  `toggle.style`.

Adding a variant needs a Webox release. That is the price of "no free-form drawing", and it keeps every look tested,
accessible and fast. Plugin authors propose variants through issues. Section 4 covers the gaps in between.

---

## 4. Custom content: declarative widgets

For content beyond variants (a status widget in the header, a panel with plugin data, extra buttons in a row), a plugin
sends a **view**: a small widget tree that Webox renders with its own primitives. Actions come back to the plugin as
events. No egui crosses the ABI.

```rust
pub(crate) enum W {
    Row { gap: LenRef, align: Align, children: Vec<W> },
    Column { gap: LenRef, align: Align, children: Vec<W> },
    Text { text: TextSrc, role: FontRole, color: Option<TokenId>, max_lines: u8 },
    Icon { name: IconRef, size: f32, color: Option<TokenId> },
    Image { asset: AssetId, size: Vec2, shape: Shape },        // package files only, never URLs
    Button { label: Option<TextSrc>, icon: Option<IconRef>, style: ButtonStyle, action: WAction, tooltip: Option<String> },
    Badge { text: TextSrc, tone: Tone },
    Spacer { size: Option<f32> },                               // None = flexible
    Separator,
    ListRow { title: TextSrc, subtitle: Option<TextSrc>, leading: Option<Box<W>>, action: WAction },
}
pub(crate) enum TextSrc { Lit(String), Bind(StateKeyId) }     // "{space.selected.title}" resolved by Webox
pub(crate) enum WAction { Plugin(String), Host(Action) }       // Host actions go through dispatch() and its guards
```

- **Where views appear**: slots (`header.actions`, `chat_header.actions`, `space_list.row.trailing` per space via a
  template, `composer.toolbar`) and a `plugin_panel` component the layout places like any other.
- **Protocol** (contract 1.6 named methods, no table change):
  - `ui.view.set {"id","tree"}`: parse, validate and compile **once**, off the frame path, into `Rc<ViewTree>`, then wake.
    Updates are coalesced to at most 30 per second per plugin.
  - `ui.view.clear {"id"}`.
  - Event `ui.action {"view","id","space"?}`: a plugin button was pressed. The plugin answers with nothing, a new view,
    or `ui.dispatch {"action":"space.open","space":"..."}`.
  - Event `ui.view.shown {"id","visible"}`, so plugins can stop work for hidden panels.
- **Bindings** keep it cheap: `TextSrc::Bind` reads a whitelisted state key at draw time. A clock or an unread count does
  not need a plugin round-trip each time it changes.
- **Limits**: depth 8, 256 nodes, 4 KB of text per view, 16 views per plugin. Text is plain (or the span format of
  `ui.dialog` / `message.keep`). There are no fonts or colours outside tokens.
- **Behaviour**: views get focus order, tooltips, keyboard activation (Enter/Space on buttons) and accessibility labels
  from the primitives, like built-in UI.

This generalises what already works: plugin settings pages, `MenuItem` lists and `ui.dialog` are host-rendered
declarative UI.

---

## 5. Plugin API

### Packaging

A **UI package** is a folder (or a `.zip` in a repository) with a manifest:

```
webox-ui.json            manifest
themes/<id>.json         themes and theme patches
layouts/<id>.json        layouts and layout patches
fonts/*.ttf|otf
icons/<set>/*.svg
images/*.png|svg         assets for views
```

```json
{
  "schema": "webox-ui/1",
  "id": "dev.example.nord",
  "name": "Nord",
  "version": "1.0.0",
  "minWebox": "0.4.0",
  "requires": { "components": { "message_list": 1, "space_list": 1 } },
  "provides": {
    "themes":  [{ "id": "nord", "file": "themes/nord.json", "kind": "theme" }],
    "patches": [{ "id": "nord-bubbles", "file": "themes/bubbles.json", "kind": "theme-patch" }],
    "layouts": []
  }
}
```

- **Data-only packages** need no native code, which makes them the safe default for themes. They are installed from a
  repository entry with `"kind": "ui"` and a single `"any"` file (the zip). This is an additive index field: older Webox
  versions skip entries without a platform file. SHA-256 pinning, the size cap and the redirect rules stay as in
  `plugin_repo`.
- **Native plugins** can ship the same folder next to their library (`<pkg>/<version>/ui/`). They can also register
  content at runtime with `ui.theme.register {"id","json"}` / `ui.layout.register`. That allows themes computed from
  settings, which accent plugins do today.
- **Format**: JSON, because `serde_json` is already a dependency. The parser accepts `//` comments and trailing commas in
  package files (open question: TOML).

### Contract and versioning

- `ABI_VERSION` stays 1. **No `PluginVTable` / `HostApi` change**: everything above is named methods and events (the
  1.6 mechanism). The contract minor goes to **1.7** only so the repository's `contract` field can mark plugins that
  need these methods. Per-name versions appear in `host.capabilities` as usual.
- The UI schema has its own version (`webox-ui/1`, `webox-ui-layout/1`). Component contracts are versioned separately
  (`message_list@1`). Packages declare what they need in `requires`, and Webox refuses with a clear message when it lacks
  a version.
- Additions (new variant option, new slot, new token) keep the version. Removing or changing the meaning of something
  creates a new id. This mirrors the existing rule for method names.
- The legacy hooks `palette` (1.0), `accents` (1.3) and `decor` (1.4) keep working. An adapter turns them into a theme
  patch at rebuild time: palette goes to role overrides, accents to the token overrides of `accents::apply`, decor to
  `ResolvedTheme::decor`. `plugins/pride-theme` must render unchanged (its shots in `scripts/pride-shots.ps1` are
  part of the parity set).

### Validation, fallback, lint

The pipeline on load and on every reload: **parse** (with line and column) → **schema** (known keys and kinds) →
**resolve** (references, `extends` chains, cycles) → **lint** → **compile**.

Lint rules:

- Errors: a `Required` component missing; a `RequiredReachable` component without a route; unsatisfiable min sizes at
  the smallest window (800 × 560); an unknown component, slot, variant or param; a reference cycle; a font or icon file
  that is missing or too large.
- Warnings: text-to-background contrast below WCAG AA for role pairs (`text1`/`card`, `text2`/`card`,
  `button-primary-text`/`button-primary-background`); density values off the 4 pt grid; unknown core tokens (they fall
  back to the default theme's value).

On any error, the **last good** version of that package stays active. On first load that is the default. Webox shows
one toast ("Layout ‘Discordish’ has an error, using the default layout") and the full list on the package's page in
**Settings > Plugins**, with `file:line` and JSON path. `webox --lint-ui <dir>` runs the same pipeline headless. It
prints the findings and exits non-zero on errors, so it can run in a plugin's CI. `--lint-ui <dir> --screenshot out.png
--sample` additionally renders the package with sample data.

### Hot reload

For development only: switched on per package on its plugin page ("Watch for changes"), or with `WEBOX_UI_DEV=<dir>`. A
background thread polls the package's file mtimes once a second (no new dependency) and posts "changed". The UI thread
rebuilds on the next frame. Failures keep the last good version and show the lint list in a small overlay. Repository
installs are never watched.

### Combining several plugins

- **One layout** is active per surface, chosen in **Settings > Erscheinungsbild > Layout** (default: "Webex").
- **One base theme**: a Momentum theme or a plugin theme, chosen in the existing theme picker. Plugin themes appear after
  the ten Momentum tiles.
- **Any number of patches** (theme patches, layout patches, legacy design plugins), applied in a user-ordered list. A
  later patch wins per key. Layout patches address nodes by `id`, with the operations `set`, `hide`, `move` (to a
  parent and index), `insert` and `slot.add`.
- The **user patch** ("Oberfläche anpassen") is always last.
- A layout patch that targets a node id missing from the active layout is skipped with a warning, so patches written
  for the default layout degrade gracefully on others.

Order of resolution: default theme → base theme (`extends` chain) → theme patches in order → mode block (dark/light) →
density overrides → HC mapping → user prefs. For layouts: base layout → layout patches in order → user patch →
validation of the result.

---

## 6. Performance budget

The rule is that the engine never costs frames. Concretely:

- No JSON parsing, `serde_json::Value` walks or C-ABI calls during a frame. This removes today's per-frame
  `palette`/`decor`/`accents` calls, the per-frame `density::set_from` and `customize::visible` JSON reads, and the
  `Vec` clones in `accent_gradient()` / `decor::stripes()`.
- No allocation for theme or layout per frame. `ResolvedTheme` is an `Arc` swapped on rebuild. Solved rects live in a
  reused buffer. Readers get slices.
- Token reads are array indices (`TokenId` consts). Remaining `tk(&str)` calls get a debug-build counter so they can be
  hunted down.
- A rebuild targets under 2 ms for theme plus layout, excluding font file I/O, which happens off-thread. It happens only
  on the triggers listed in section 1.
- Measured with `WEBOX_PROBE=1` (frames, textures, repaint causes; `src/ui/frame.rs`). An idle window must not gain
  repaints. CPU and RAM are compared against the current release and official Webex, per the project rule.
- Memory: the resolved theme is a few KB. Fonts are the only large item, and they are capped.

---

## 7. Migration plan

Each phase is one or more PRs. Each PR keeps the default look **pixel-identical** and all tests green before merge.
Effort is in agent-days of focused work, including tests and screenshots.

**Phase 0: parity harness (2–3 d).** Add `scripts/parity.ps1`, which renders a fixed scene list with `--sample
--screenshot`. The list is everything in `shots.ps1` plus themes (dark, light, jade light), densities (standard,
compact, compact view), zooms (1.0, 1.25), menus, call states (`WEBOX_SHOT_CALL`), settings pages, pride shots and the
wide rail. The script compares the results against a baseline built from `main` and reports any differing pixel with
a diff image. It runs in CI on Windows; Linux uses Selawik and has a separate baseline. Exact match is required, and
known-flaky scenes (animations) are rendered at a fixed frame.

**Phase 1: resolve once (3–4 d).** Introduce `ResolvedTheme` with `gen`. Move the plugin `palette`/`accents`/`decor`
calls, `set_accent_overrides`, the decor conversion and `density::set_from` into the rebuild, triggered by settings or
plugin changes. Remove the per-frame clones. This brings an immediate CPU win with no visual change.

**Phase 2: token ids and the full theme value set (8–10 d).** Generate `TokenId` consts and `tok!`. Move the ~250
literal `hex(0x…)` colours to component tokens whose defaults equal the literals (for example the emerald call buttons in
`conversation.rs`). Move the ~380 font literals to `FontRole`s, and radii, strokes, shadows and motion to tokens. Fold
`Metrics` into the theme. Mechanical but wide, so it is done component by component in this order: chrome/header → rail
→ space list → conversation banner and tabs → message list → composer → menus and dialogs → settings → pages → call
windows.

**Phase 3: action bus and state keys (4–5 d).** `Action` + `App::dispatch` with guards. Route the rail, header,
banner buttons, composer toolbar and shortcut commands through it. Add the `StateKey` reader and `StateBits`.

**Phase 4: layout engine for the main surface (8–12 d).** `Node`, compile, solve, render. Replace the panel sequence in
`frame.rs`, `messaging.rs`, `page_kit.rs` and `settings/mod.rs` with the default tree. Generalise `list_splitter` to
`Split` splitters, keeping the persisted `list_w`. Make the rail width animation a node size driven by the motion token.
Move `decor::chrome`'s panel lookups to solved rects.

**Phase 5: componentise the conversation (8–10 d).** Split `conversation()` into `chat_header`, `space_tabs`,
`message_list` and `composer` behind `ComponentSpec`s. Keep the composite. Move the `customize.rs` groups into slots plus
the user patch, with the stored keys unchanged.

**Phase 6: variants (8–10 d).** `message_list` bubbles and compact, avatar shapes, the `space_list` row variants, the
rail's horizontal and server-bar modes, the chat header bar, composer toolbar positions and button styles. Each variant
gets parity shots of its own (new baselines) and runs in the behaviour suite.

**Phase 7: packages, validation, lint and hot reload (6–8 d).** Manifest, loader, repository `kind: "ui"`, the legacy
adapter, the user-ordered patch list, `--lint-ui`, the error UI, the dev watcher, the Settings > Layout picker and the
`docs/plugins.md` reference sections (one per component contract).

**Phase 8: views and plugin panels (5–6 d).** The `W` renderer on shared primitives, `ui.view.*` and `ui.action`,
bindings and limits.

**Phase 9: call surfaces (6–8 d).** The call window and dock as layout surfaces. The plugin controls where the stage,
roster, control bar and stats go, and the stage tile arrangement variants (grid, speaker, filmstrip position). Tile
contents stay hard-coded.

**Phase 10: example plugins and the alternative-layout suite (4–5 d).** Ship examples A and B in `plugins/` as workspace
members (like `pride-theme`) and run the behaviour suite against B in CI.

**Total: about 65–85 agent-days.** Phases 0–1 are independent and worth doing first even if the rest waits. Phases 2 and
3 can run in parallel (different files). Phases 4 and 5 depend on 2 and 3.

### What stays hard-coded, and why

- **Call media internals**: video decode, texture upload, the per-tile drawing of video, names, mute and active-speaker
  states, the screen-share viewer. These are timing- and memory-sensitive and tightly bound to `src/media`. Layouts
  choose the arrangement, not the tile renderer.
- **Text layout and editing**: rich-text measurement and the row cache (`messages/rows.rs` `ROWS`, `header_height`),
  the composer's text editing, IME and formatting. Styles come from tokens, but the algorithms stay put. The row cache is
  keyed by content plus the theme `gen`.
- **Window and OS integration**: frameless edges and hit-testing (`chrome::edge_strips`, `edge_overlay`), the sign-in
  webview, toast and tray windows (`src/platform/notify`), the screen-capture overlay and the incoming-call toast. These
  must match OS behaviour and are not "layout".
- **Global interaction machinery**: the menu stack, Escape order, the shortcut registry and focus traversal (see
  "Behaviour and coherence").
- **High-contrast mapping**: accessibility must not be overridable by a theme by accident.
- **Pickers with their own physics**: the emoji and GIF picker grids. They are styleable through tokens; their internal
  layout stays.

---

## 8. Testing

- **Pixel parity** (Phase 0 harness) on every PR that touches `src/ui`.
- **Unit tests**: resolution (references, `extends`, cycles, patch order, mode selection), every lint rule, compile and
  solve (min/max, ratios, collapse order, width classes), the legacy adapter (`accents_touch_only_accent_roles` and
  `api_decor_is_sanitised` keep passing through the adapter), view limits and bindings, and a "no allocation per frame"
  test that counts allocations across 100 idle frames with a counting global allocator in a test binary.
- **Behaviour suite**: headless egui frames (`egui::Context::run` with injected `RawInput`, the pattern in
  `src/ui/widgets.rs` tests and `shot_input.rs`) against `--sample` data. It is parameterised over layouts, `[default,
  example_b]`, and scenarios:
  - keyboard: Tab order matches tree order; shortcuts (Ctrl+F search, the PreviousSpace / NextSpace commands, Ctrl+Shift+F
    favourite) work; Escape closes menu → dialog → overlay;
  - selection sync: opening a space from the list selects the chat; `nav.back` restores both; unread clears;
  - menus: the row menu, message context menu and plugin menu entries open and act in both layouts;
  - composer: typing with nothing focused goes to the composer (`type_focus.rs`); send, reply, edit and drafts per space;
  - calls: start from the header, then the call window or dock appears, and hang-up returns; a closed space refuses calls
    with the same toast;
  - reachability: Settings is reachable; a collapsed list has a toggle.
- **Lint in CI** for the in-repo example packages.

---

## 9. Example plugins

### A. Recolour and fonts: "Nord"

Data-only. It changes colours and fonts, rounds a few things more and uses bubbles. The layout is the default.

`webox-ui.json`:

```json
{
  "schema": "webox-ui/1", "id": "dev.example.nord", "name": "Nord", "version": "1.0.0", "minWebox": "0.4.0",
  "requires": { "components": { "message_list": 1 } },
  "provides": { "themes": [{ "id": "nord", "file": "themes/nord.json", "kind": "theme" }] }
}
```

`themes/nord.json`:

```json
{
  "schema": "webox-ui-theme/1",
  "id": "nord",
  "name": "Nord",
  "extends": "MomentumDark",
  "high_contrast": "inherit",
  "fonts": {
    "files": { "inter": ["fonts/Inter-Regular.ttf"], "inter-sb": ["fonts/Inter-SemiBold.ttf"] },
    "families": { "body": ["inter", "@system"], "semibold": ["inter-sb", "@system-semibold"] },
    "scale": { "body": 14, "secondary": 12.5, "banner": 19 }
  },
  "modes": {
    "dark": {
      "tokens": {
        "theme-gradient-primary-0-background": "#2E3440",
        "theme-gradient-primary-1-background": "#2E3440",
        "theme-gradient-secondary-0-background": "#3B4252",
        "theme-gradient-secondary-1-background": "#353B49",
        "text-primary": "#ECEFF4",
        "text-secondary": "#D8DEE9 @ 70%",
        "mainList-indicator": "#88C0D0",
        "button-hyperlink-text": "#81A1C1",
        "listItem-primary:checked": "#434C5E",
        "listItem-primary:hovered": "#434C5E @ 60%"
      },
      "components": {
        "message_list.bubble.background.own": "#5E81AC",
        "message_list.bubble.background.other": "#434C5E",
        "message_list.bubble.text.own": "#ECEFF4",
        "chat_header.call_button.background": "#A3BE8C"
      }
    },
    "light": {
      "extends": "MomentumDefault",
      "tokens": { "theme-gradient-secondary-0-background": "#ECEFF4", "text-primary": "#2E3440", "mainList-indicator": "#5E81AC" }
    }
  },
  "lengths": { "card.radius": 14, "space_list.row.radius": 10, "button.radius": 8 },
  "motion": { "rail.expand": { "secs": 0.22, "ease": "cubic_out" } },
  "defaults": { "message_list": { "variants": { "style": "bubbles", "own_align": "right", "avatar": "rounded" } } }
}
```

What this shows for **behaviour and coherence**: only tokens, lengths, fonts and a declared variant change. Selection,
read state, menus, shortcuts and scrolling are the same code paths. If `fonts/Inter-SemiBold.ttf` is missing, lint
reports it, the `semibold` family falls back to `@system-semibold`, and the theme still applies. The contrast lint checks
`#D8DEE9 @ 70%` on `#3B4252`. A theme with no `light` block would keep the Momentum light colours in light mode.

### B. A fully different layout: "Discordish"

This package moves navigation to a server bar of favourite spaces and teams, adds a channel list, puts the chat in the
middle and a members panel on the right, and gives a compact single column below 720 pt. It also uses a native
plugin's view for a status widget. To show that it combines with themes, it ships no colours.

`layouts/discordish.json`:

```json
{
  "schema": "webox-ui-layout/1",
  "id": "discordish",
  "name": "Discordish",
  "panels": { "members": { "default_open": true } },
  "surfaces": { "main": {
    "switch": "window.width_class",
    "cases": {
      "narrow": {
        "id": "single", "dock": "top", "size": { "pref": 40 },
        "child": { "slot": "header", "variants": { "compact": true } },
        "rest": { "switch": "space.selected", "cases": {
          "none": { "slot": "space_list", "variants": { "row": "one_line" } },
          "*": { "id": "chat-col", "stack": "column", "children": [
            { "slot": "chat_header", "variants": { "size": "bar", "actions": "icons" },
              "slots": { "leading": [{ "button": { "icon": "arrow-left-regular", "action": "nav.back", "tooltip": "Back" } }] } },
            { "slot": "message_list", "grow": true, "variants": { "style": "compact", "avatar": "circle" } },
            { "slot": "composer", "variants": { "toolbar": "inline", "shape": "pill" } } ] } } } }
    },
    "default": {
      "id": "server-bar", "dock": "left", "size": { "pref": 72 },
      "child": { "slot": "rail", "variants": { "source": "teams_and_spaces", "item": "square", "labels": "never" } },
      "rest": {
        "id": "top", "dock": "top", "size": { "pref": 40 },
        "child": { "slot": "header", "variants": { "compact": true },
                   "slots": { "actions": [{ "view": "dev.example.discordish/status" }] } },
        "rest": { "switch": "nav.page", "cases": {
          "messaging": { "split": "row", "id": "main-split", "items": [
            { "size": { "min": 200, "pref": 240, "max": 320, "resizable": true },
              "node": { "slot": "space_list", "variants": { "row": "one_line", "sections": "flat", "preview": false } } },
            { "size": { "min": 420 }, "node": { "stack": "column", "id": "chat", "children": [
              { "slot": "chat_header", "variants": { "size": "bar", "actions": "icons" },
                "slots": { "actions": [{ "button": { "icon": "people-regular", "action": "panel.toggle:members", "tooltip": "Members" } }] } },
              { "slot": "message_list", "grow": true, "variants": { "style": "compact", "timestamp": "hover", "hover_bar": "inline" } },
              { "slot": "composer", "variants": { "toolbar": "inline", "shape": "card" } } ] } },
            { "size": { "min": 200, "pref": 240, "max": 300, "collapse_below": 1000 },
              "when": { "key": "panel.members.open" },
              "node": { "slot": "roster", "variants": { "source": "space_members" } } } ] },
          "settings": { "split": "row", "items": [
            { "size": { "pref": 220 }, "node": { "slot": "settings_nav" } },
            { "node": { "slot": "settings_page" } } ] },
          "*": { "slot": "page" } } } } } },
  "call": { "surfaces": { "call_window": {
    "split": "row", "items": [
      { "node": { "slot": "call_stage", "variants": { "arrangement": "speaker", "filmstrip": "bottom" } } },
      { "size": { "pref": 260, "collapse_below": 900 }, "when": { "key": "panel.roster.open" }, "node": { "slot": "roster" } } ] } } },
  "reach": { "settings": "rail.bottom.settings", "space_list": "nav.back (narrow)" }
}
```

The optional native part (`dev.example.discordish`, contract 1.7) keeps a small status widget up to date:

```rust
impl Plugin for Discordish {
    const EVENTS: &'static str = "ui.action";
    fn start(&mut self, host: Host) {
        host.call("ui.view.set", r#"{"id":"status","tree":
          {"row":{"gap":"{space.xs}","children":[
            {"badge":{"text":{"bind":"unread.count"},"tone":"accent"}},
            {"button":{"icon":"headset-regular","style":"ghost","action":"plugin:toggle-dnd","tooltip":"Do not disturb"}}]}}}"#);
        self.host = Some(host);
    }
    fn on_event(&mut self, name: &str, payload: &str) -> Option<String> {
        if name == "ui.action" && payload.contains("\"toggle-dnd\"") {
            // Same rules as the status menu: goes through dispatch() on the host.
            self.host.as_ref()?.call("ui.dispatch", r#"{"action":"presence.set","status":"dnd"}"#);
        }
        None
    }
}
```

What this shows for **behaviour and coherence**:

- Opening a space in the channel list dispatches `space.open`. The chat column, the members panel (`space_members` reads
  `space.selected`) and the server bar's selection update together, as in the default layout.
- The narrow case has no list beside the chat. `space_list` is `RequiredReachable` through `nav.back` from the chat's
  back button, and lint verifies that route. Settings stays reachable through the server bar's bottom item.
- Escape, Ctrl+F search, the PreviousSpace / NextSpace commands, drag-and-drop onto the composer, typing-to-composer and the
  message context menu are unchanged, because those belong to components and global rules. Tab order follows the
  tree: server bar → header → channel list → chat header → messages → composer → members.
- The members toggle is a host action (`panel.toggle:members`). The status widget's button goes back to the plugin, which
  uses `ui.dispatch` and is subject to the same presence rules as the built-in menu.
- If a future Webox drops `rail.source = teams_and_spaces`, lint fails, the default layout is used, and an error names
  the variant and the contract version.
- The behaviour suite runs against this file in CI.

---

## 10. Risks and open questions

**Risks**

- **Parity drift during Phase 2.** Hundreds of literal sites move. Mitigations: the exact-pixel harness, small PRs per
  component, and token defaults that equal the literals.
- **egui focus order** is creation order. Overlays and floating panes drawn late can break the expected Tab order, so the
  engine needs an explicit focus order per tree, possibly via `Memory::interested_in_focus` ordering. This needs a spike
  in Phase 4.
- **Combinatorial testing.** Variants multiply. The suite covers each variant once in a representative layout, not every
  product.
- **Font licensing and size** for plugin fonts. Webox cannot vet licences, so the repository policy should require OFL
  or similar. The 24 MB package cap limits RAM.
- **Composite components** (conversation, call window) have hidden geometric couplings: the jump button sits above the
  composer, and reply banners push the list. Splitting them may surface bugs that the fixed layout hid.
- **Discord-style server bar** needs a new data path (favourite spaces and teams as icons, with unread badges). It is
  product work, not just layout.
- **Security of data-only packages**: low (no code), but SVG and font parsing are attack surface. Use the existing
  renderer with limits, strip external references and parse fonts off-thread.

**Open questions**

1. JSON with comments, or TOML for package files? TOML reads better for themes but adds a dependency.
2. Should data-only UI packages be the **only** way to ship layouts, with native plugins limited to
   `ui.theme.register` for computed themes? That would be simpler and safer.
3. How far into Settings may a layout go? This design lets it place `settings_nav` / `settings_page` but not restyle
   individual setting rows beyond tokens.
4. Should the call window and dock layouts belong to the same package choice as the main layout, or be chosen
   separately?
5. Is a per-space layout ("focus mode" for one space) wanted, or is one global layout enough?
6. Should plugin themes be allowed under Windows high contrast at all (`high_contrast` opt-in), or always be ignored
   there?
7. Should there be a public, versioned list of state keys and actions in `docs/plugins.md` from Phase 3 on, even before
   layouts ship, so that `ui.dispatch` can arrive early for existing plugins?
8. What do we do with `Th` itself long-term? Keep it as the fast "roles" view, or replace it with `ResolvedTheme`
   accessors?

## Decisions on the open questions (2026-11-25)

1. **File format:** JSON. `serde_json` is already a dependency; TOML would add one for no gain at runtime. Plugin authors may write TOML and convert at build time if they like.
2. **Layouts are data-only packages.** Native plugins may compute themes (palette, accents) but cannot ship a layout tree from code. Data is easy to validate, lint and hot-reload.
3. **Settings:** a layout can place the settings page and choose its navigation style, but not restructure the pages themselves (first version).
4. **Call window and dock layouts are chosen separately** from the main layout, with the main layout's package as the default when it provides them.
5. **Per-space layouts: no** (not until someone needs them).
6. **High contrast wins:** under Windows high contrast, plugin colours are ignored and the high-contrast mapping applies; layouts still apply.
7. **Publish the action and state-key list early, from P3**, marked unstable until P7.
8. **`Th` stays** as the fast resolved colour struct, filled once per theme change.

