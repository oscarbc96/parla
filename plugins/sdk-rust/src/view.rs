//! View trees: the WIT types plus nested builders that flatten
//! into the flat node list the host draws, and [`validate`], the host's
//! eight tree rules, so a plugin can check a tree in its own tests.
//!
//! ```ignore
//! use parla_plugin_sdk::view::{button, column, heading, text};
//! let tree = column()
//!     .child(heading("Standup"))
//!     .child(text("Ready?"))
//!     .child(button("go", "Go"))
//!     .build();
//! ```

pub use crate::bindings::parla::plugin::view::*;
use crate::types::Error;

/// Most nodes in one tree (rule 8).
pub const MAX_NODES: usize = 2_000;
/// Most bytes in one tree by [`size`] (rule 8).
pub const MAX_BYTES: usize = 256 * 1024;
const NODE_BYTES: usize = 16;
const ELEMENT_BYTES: usize = 4;

/// A node with its children, flattened by [`Builder::build`]. Kinds with
/// their own setters have their own builder type ([`ButtonBuilder`], ...),
/// which converts into this one.
#[derive(Clone, Debug)]
pub struct Builder {
    id: Option<String>,
    kind: NodeKind,
    children: Vec<Builder>,
}

impl Builder {
    /// Sets the node id (needed for events and `ui.patch`).
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Appends a child.
    pub fn child(mut self, child: impl Into<Builder>) -> Self {
        self.children.push(child.into());
        self
    }

    /// Appends children in order.
    pub fn children<B: Into<Builder>>(mut self, children: impl IntoIterator<Item = B>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// Flattens the tree in document order: each node follows its parent,
    /// which it names by index.
    pub fn build(self) -> ViewTree {
        let mut nodes = Vec::new();
        flatten(self, None, &mut nodes);
        ViewTree { nodes }
    }
}

fn flatten(b: Builder, parent: Option<u32>, out: &mut Vec<Node>) {
    let index = out.len() as u32;
    out.push(Node {
        id: b.id,
        parent,
        kind: b.kind,
    });
    for c in b.children {
        flatten(c, Some(index), out);
    }
}

/// Builder types for kinds with setters: each wraps a [`Builder`] and has
/// `id`, `build` and `Into<Builder>`; the kind-specific setters follow.
macro_rules! typed_builder {
    ($( $(#[$m:meta])* $name:ident($variant:ident); )*) => {
        $(
            $(#[$m])*
            #[derive(Clone, Debug)]
            pub struct $name(Builder);

            impl $name {
                pub fn id(self, id: impl Into<String>) -> Self {
                    Self(self.0.id(id))
                }
                pub fn build(self) -> ViewTree {
                    self.0.build()
                }
                fn edit(mut self, f: impl FnOnce(&mut NodeKind)) -> Self {
                    f(&mut self.0.kind);
                    self
                }
            }

            impl From<$name> for Builder {
                fn from(b: $name) -> Builder {
                    b.0
                }
            }
        )*
    };
}

typed_builder! {
    /// [`button`]: `look`, `disabled`.
    ButtonBuilder(Button);
    /// [`input`] and [`textarea`]: `placeholder`, `value`.
    InputBuilder(Input);
    /// [`select`] and [`segmented`]: `value`.
    SelectBuilder(Select);
    /// [`stat`]: `note`.
    StatBuilder(Stat);
    /// [`progress`]: `label`.
    ProgressBuilder(Progress);
    /// [`callout`]: `title`.
    CalloutBuilder(Callout);
    /// [`empty`]: `text`.
    EmptyBuilder(Empty);
    /// [`table`]: `selectable`, `load_more` (both need an id).
    TableBuilder(Table);
    /// [`list`]: `selectable`, `load_more` (both need an id), `child`.
    ListBuilder(List);
}

impl ButtonBuilder {
    pub fn look(self, look: ButtonLook) -> Self {
        self.edit(|k| {
            if let NodeKind::Button(b) = k {
                b.look = look;
            }
        })
    }
    pub fn disabled(self, disabled: bool) -> Self {
        self.edit(|k| {
            if let NodeKind::Button(b) = k {
                b.disabled = disabled;
            }
        })
    }
}

impl InputBuilder {
    pub fn placeholder(self, placeholder: impl Into<String>) -> Self {
        let p = placeholder.into();
        self.edit(|k| {
            if let NodeKind::Input(t) | NodeKind::Textarea(t) = k {
                t.placeholder = Some(p);
            }
        })
    }
    pub fn value(self, value: impl Into<String>) -> Self {
        let v = value.into();
        self.edit(|k| {
            if let NodeKind::Input(t) | NodeKind::Textarea(t) = k {
                t.value = v;
            }
        })
    }
}

impl SelectBuilder {
    /// The chosen option's value.
    pub fn value(self, value: impl Into<String>) -> Self {
        let v = value.into();
        self.edit(|k| {
            if let NodeKind::Select(s) | NodeKind::Segmented(s) = k {
                s.value = Some(v);
            }
        })
    }
}

impl StatBuilder {
    pub fn note(self, note: impl Into<String>) -> Self {
        let n = note.into();
        self.edit(|k| {
            if let NodeKind::Stat(s) = k {
                s.note = Some(n);
            }
        })
    }
}

impl ProgressBuilder {
    pub fn label(self, label: impl Into<String>) -> Self {
        let l = label.into();
        self.edit(|k| {
            if let NodeKind::Progress(p) = k {
                p.label = Some(l);
            }
        })
    }
}

impl CalloutBuilder {
    pub fn title(self, title: impl Into<String>) -> Self {
        let t = title.into();
        self.edit(|k| {
            if let NodeKind::Callout(c) = k {
                c.title = Some(t);
            }
        })
    }
}

impl EmptyBuilder {
    pub fn text(self, text: impl Into<String>) -> Self {
        let t = text.into();
        self.edit(|k| {
            if let NodeKind::Empty(e) = k {
                e.text = Some(t);
            }
        })
    }
}

impl TableBuilder {
    pub fn selectable(self, selectable: bool) -> Self {
        self.edit(|k| {
            if let NodeKind::Table(t) = k {
                t.selectable = selectable;
            }
        })
    }
    pub fn load_more(self, load_more: bool) -> Self {
        self.edit(|k| {
            if let NodeKind::Table(t) = k {
                t.load_more = load_more;
            }
        })
    }
}

impl ListBuilder {
    pub fn selectable(self, selectable: bool) -> Self {
        self.edit(|k| {
            if let NodeKind::List(l) = k {
                l.selectable = selectable;
            }
        })
    }
    pub fn load_more(self, load_more: bool) -> Self {
        self.edit(|k| {
            if let NodeKind::List(l) = k {
                l.load_more = load_more;
            }
        })
    }
    /// Appends a row.
    pub fn child(self, child: impl Into<Builder>) -> Self {
        Self(self.0.child(child))
    }
    pub fn children<B: Into<Builder>>(self, children: impl IntoIterator<Item = B>) -> Self {
        Self(self.0.children(children))
    }
}

/// Any node kind, for kinds without a helper below.
pub fn node(kind: NodeKind) -> Builder {
    Builder {
        id: None,
        kind,
        children: Vec::new(),
    }
}

fn with_id(id: impl Into<String>, kind: NodeKind) -> Builder {
    node(kind).id(id)
}

fn strings<S: Into<String>>(items: impl IntoIterator<Item = S>) -> Vec<String> {
    items.into_iter().map(Into::into).collect()
}

// Layout.

pub fn column() -> Builder {
    node(NodeKind::Column)
}
pub fn row() -> Builder {
    node(NodeKind::Row)
}
pub fn grid(columns: u8) -> Builder {
    node(NodeKind::Grid(columns))
}
pub fn card() -> Builder {
    node(NodeKind::Card)
}
pub fn divider() -> Builder {
    node(NodeKind::Divider)
}
pub fn spacer() -> Builder {
    node(NodeKind::Spacer)
}
/// Tabs: add one child per label, the body of that tab.
pub fn tabs<S: Into<String>>(
    id: impl Into<String>,
    labels: impl IntoIterator<Item = S>,
) -> Builder {
    with_id(id, NodeKind::Tabs(strings(labels)))
}
/// Main plus side: add exactly two children.
pub fn split() -> Builder {
    node(NodeKind::Split)
}

// Text.

/// A plain span, for [`rich`].
pub fn span(text: impl Into<String>) -> Span {
    Span {
        text: text.into(),
        bold: false,
        italic: false,
        code: false,
        link: None,
    }
}
pub fn heading(text: impl Into<String>) -> Builder {
    node(NodeKind::Heading(text.into()))
}
/// One plain span of text.
pub fn text(text: impl Into<String>) -> Builder {
    rich([span(text)])
}
/// Text from styled spans.
pub fn rich(spans: impl IntoIterator<Item = Span>) -> Builder {
    node(NodeKind::Text(spans.into_iter().collect()))
}
pub fn label(text: impl Into<String>) -> Builder {
    node(NodeKind::Label(text.into()))
}
pub fn caption(text: impl Into<String>) -> Builder {
    node(NodeKind::Caption(text.into()))
}
pub fn code(text: impl Into<String>, language: Option<&str>) -> Builder {
    node(NodeKind::Code(CodeBlock {
        text: text.into(),
        language: language.map(Into::into),
        line_numbers: false,
        highlight: None,
    }))
}

// Data.

pub fn badge(text: impl Into<String>, tone: Tone) -> Builder {
    node(NodeKind::Badge(BadgeBlock {
        text: text.into(),
        tone,
    }))
}
pub fn chip(text: impl Into<String>) -> Builder {
    node(NodeKind::Chip(text.into()))
}
pub fn stat(label: impl Into<String>, value: impl Into<String>) -> StatBuilder {
    StatBuilder(node(NodeKind::Stat(StatBlock {
        label: label.into(),
        value: value.into(),
        note: None,
    })))
}
/// `value` in 0.0 to 1.0; `None` is indeterminate.
pub fn progress(value: Option<f32>) -> ProgressBuilder {
    ProgressBuilder(node(NodeKind::Progress(ProgressBlock {
        value,
        label: None,
    })))
}
pub fn avatar(name: impl Into<String>, url: Option<&str>) -> Builder {
    node(NodeKind::Avatar(AvatarBlock {
        name: name.into(),
        url: url.map(Into::into),
    }))
}
/// `src` is a bundled `assets/` path or an https URL on a granted host.
pub fn image(src: impl Into<String>, alt: impl Into<String>) -> Builder {
    node(NodeKind::Image(ImageBlock {
        src: src.into(),
        alt: alt.into(),
        width: None,
        height: None,
    }))
}
pub fn icon(name: impl Into<String>, label: impl Into<String>) -> Builder {
    node(NodeKind::Icon(IconBlock {
        name: name.into(),
        label: label.into(),
        tone: None,
    }))
}
/// A table: `table(["Name", "Age"], [["Ada", "36"]])`.
pub fn table<C, R, S>(columns: C, rows: R) -> TableBuilder
where
    C: IntoIterator,
    C::Item: Into<String>,
    R: IntoIterator,
    R::Item: IntoIterator<Item = S>,
    S: Into<String>,
{
    TableBuilder(node(NodeKind::Table(TableBlock {
        columns: strings(columns),
        rows: rows.into_iter().map(strings).collect(),
        selectable: false,
        load_more: false,
    })))
}
/// A virtual list; its children are the rows.
pub fn list() -> ListBuilder {
    ListBuilder(node(NodeKind::List(ListBlock {
        selectable: false,
        load_more: false,
    })))
}

// Input.

fn text_input(label: impl Into<String>) -> TextInput {
    TextInput {
        label: label.into(),
        value: String::new(),
        placeholder: None,
    }
}
pub fn input(id: impl Into<String>, label: impl Into<String>) -> InputBuilder {
    InputBuilder(with_id(id, NodeKind::Input(text_input(label))))
}
pub fn textarea(id: impl Into<String>, label: impl Into<String>) -> InputBuilder {
    InputBuilder(with_id(id, NodeKind::Textarea(text_input(label))))
}
pub fn select(
    id: impl Into<String>,
    label: impl Into<String>,
    options: Vec<Choice>,
) -> SelectBuilder {
    SelectBuilder(with_id(
        id,
        NodeKind::Select(SelectInput {
            label: label.into(),
            options,
            value: None,
        }),
    ))
}
pub fn segmented(
    id: impl Into<String>,
    label: impl Into<String>,
    options: Vec<Choice>,
) -> SelectBuilder {
    SelectBuilder(with_id(
        id,
        NodeKind::Segmented(SelectInput {
            label: label.into(),
            options,
            value: None,
        }),
    ))
}
pub fn checkbox(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Builder {
    with_id(
        id,
        NodeKind::Checkbox(ToggleInput {
            label: label.into(),
            checked,
        }),
    )
}
pub fn switch(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Builder {
    with_id(
        id,
        NodeKind::Switch(ToggleInput {
            label: label.into(),
            checked,
        }),
    )
}
pub fn button(id: impl Into<String>, label: impl Into<String>) -> ButtonBuilder {
    ButtonBuilder(with_id(
        id,
        NodeKind::Button(ButtonBlock {
            label: label.into(),
            look: ButtonLook::Default,
            disabled: false,
        }),
    ))
}
/// A form: its inputs' values arrive together in the `submit` event.
pub fn form(id: impl Into<String>) -> Builder {
    with_id(id, NodeKind::Form)
}
pub fn conversation_picker(id: impl Into<String>, label: impl Into<String>) -> Builder {
    with_id(
        id,
        NodeKind::ConversationPicker(Picker {
            label: label.into(),
        }),
    )
}
pub fn user_picker(id: impl Into<String>, label: impl Into<String>) -> Builder {
    with_id(
        id,
        NodeKind::UserPicker(Picker {
            label: label.into(),
        }),
    )
}

// Feedback.

pub fn callout(tone: Tone, text: impl Into<String>) -> CalloutBuilder {
    CalloutBuilder(node(NodeKind::Callout(CalloutBlock {
        tone,
        title: None,
        text: text.into(),
    })))
}
pub fn empty(title: impl Into<String>) -> EmptyBuilder {
    EmptyBuilder(node(NodeKind::Empty(EmptyBlock {
        title: title.into(),
        text: None,
    })))
}
pub fn spinner() -> Builder {
    node(NodeKind::Spinner)
}
pub fn skeleton() -> Builder {
    node(NodeKind::Skeleton)
}

fn kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Column => "column",
        NodeKind::Row => "row",
        NodeKind::Grid(_) => "grid",
        NodeKind::Card => "card",
        NodeKind::Divider => "divider",
        NodeKind::Spacer => "spacer",
        NodeKind::Tabs(_) => "tabs",
        NodeKind::Split => "split",
        NodeKind::Heading(_) => "heading",
        NodeKind::Text(_) => "text",
        NodeKind::Label(_) => "label",
        NodeKind::Caption(_) => "caption",
        NodeKind::Code(_) => "code",
        NodeKind::Diff(_) => "diff",
        NodeKind::Table(_) => "table",
        NodeKind::List(_) => "list",
        NodeKind::Tree(_) => "tree",
        NodeKind::Stat(_) => "stat",
        NodeKind::Progress(_) => "progress",
        NodeKind::Badge(_) => "badge",
        NodeKind::Chip(_) => "chip",
        NodeKind::Avatar(_) => "avatar",
        NodeKind::Image(_) => "image",
        NodeKind::Icon(_) => "icon",
        NodeKind::Input(_) => "input",
        NodeKind::Textarea(_) => "textarea",
        NodeKind::Select(_) => "select",
        NodeKind::Checkbox(_) => "checkbox",
        NodeKind::Switch(_) => "switch",
        NodeKind::Segmented(_) => "segmented",
        NodeKind::Button(_) => "button",
        NodeKind::Form => "form",
        NodeKind::ConversationPicker(_) => "conversation-picker",
        NodeKind::UserPicker(_) => "user-picker",
        NodeKind::Callout(_) => "callout",
        NodeKind::Empty(_) => "empty",
        NodeKind::Spinner => "spinner",
        NodeKind::Skeleton => "skeleton",
    }
}

fn is_container(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Column
            | NodeKind::Row
            | NodeKind::Grid(_)
            | NodeKind::Card
            | NodeKind::Tabs(_)
            | NodeKind::Split
            | NodeKind::Form
            | NodeKind::List(_)
    )
}

fn needs_id(kind: &NodeKind) -> bool {
    match kind {
        NodeKind::Input(_)
        | NodeKind::Textarea(_)
        | NodeKind::Select(_)
        | NodeKind::Checkbox(_)
        | NodeKind::Switch(_)
        | NodeKind::Segmented(_)
        | NodeKind::Button(_)
        | NodeKind::Form
        | NodeKind::ConversationPicker(_)
        | NodeKind::UserPicker(_)
        | NodeKind::Tabs(_)
        | NodeKind::Tree(_) => true,
        NodeKind::Table(t) => t.selectable || t.load_more,
        NodeKind::List(l) => l.selectable || l.load_more,
        _ => false,
    }
}

/// The accessible name an input, button or icon must have (rule 9).
fn accessible_label(kind: &NodeKind) -> Option<&str> {
    Some(match kind {
        NodeKind::Input(t) | NodeKind::Textarea(t) => &t.label,
        NodeKind::Select(s) | NodeKind::Segmented(s) => &s.label,
        NodeKind::Checkbox(t) | NodeKind::Switch(t) => &t.label,
        NodeKind::ConversationPicker(p) | NodeKind::UserPicker(p) => &p.label,
        NodeKind::Button(b) => &b.label,
        NodeKind::Icon(i) => &i.label,
        _ => return None,
    })
}

/// Rule 10: a bundled `assets/` path without `..`, or an https URL.
fn image_source_ok(src: &str) -> bool {
    if let Some(rest) = src.strip_prefix("https://") {
        return !rest.is_empty();
    }
    match src.strip_prefix("assets/") {
        Some(path) => {
            !path.is_empty()
                && path
                    .split('/')
                    .all(|seg| !seg.is_empty() && seg != ".." && seg != ".")
        }
        None => false,
    }
}

fn opt(s: &Option<String>) -> usize {
    s.as_ref().map_or(0, String::len)
}

/// A list element: 4 bytes plus its content.
fn el(content: usize) -> usize {
    ELEMENT_BYTES + content
}

fn text_input_size(t: &TextInput) -> usize {
    t.label.len() + t.value.len() + opt(&t.placeholder)
}

fn select_size(s: &SelectInput) -> usize {
    s.label.len()
        + opt(&s.value)
        + s.options
            .iter()
            .map(|c| el(c.value.len() + c.label.len()))
            .sum::<usize>()
}

/// One node's size by rule 8: 16, the id, every string, 4 per list element.
fn node_size(n: &Node) -> usize {
    let kind = match &n.kind {
        NodeKind::Column
        | NodeKind::Row
        | NodeKind::Grid(_)
        | NodeKind::Card
        | NodeKind::Divider
        | NodeKind::Spacer
        | NodeKind::Split
        | NodeKind::Form
        | NodeKind::List(_)
        | NodeKind::Spinner
        | NodeKind::Skeleton => 0,
        NodeKind::Tabs(labels) => labels.iter().map(|l| el(l.len())).sum(),
        NodeKind::Heading(s) | NodeKind::Label(s) | NodeKind::Caption(s) | NodeKind::Chip(s) => {
            s.len()
        }
        NodeKind::Text(spans) => spans.iter().map(|s| el(s.text.len() + opt(&s.link))).sum(),
        NodeKind::Code(c) => c.text.len() + opt(&c.language),
        NodeKind::Diff(d) => d
            .hunks
            .iter()
            .map(|h| el(h.header.len() + h.lines.iter().map(|l| el(l.text.len())).sum::<usize>()))
            .sum(),
        NodeKind::Table(t) => {
            t.columns.iter().map(|c| el(c.len())).sum::<usize>()
                + t.rows
                    .iter()
                    .map(|r| el(r.iter().map(|c| el(c.len())).sum()))
                    .sum::<usize>()
        }
        NodeKind::Tree(entries) => entries.iter().map(|e| el(e.label.len())).sum(),
        NodeKind::Stat(s) => s.label.len() + s.value.len() + opt(&s.note),
        NodeKind::Progress(p) => opt(&p.label),
        NodeKind::Badge(b) => b.text.len(),
        NodeKind::Avatar(a) => a.name.len() + opt(&a.url),
        NodeKind::Image(i) => i.src.len() + i.alt.len(),
        NodeKind::Icon(i) => i.name.len() + i.label.len(),
        NodeKind::Input(t) | NodeKind::Textarea(t) => text_input_size(t),
        NodeKind::Select(s) | NodeKind::Segmented(s) => select_size(s),
        NodeKind::Checkbox(t) | NodeKind::Switch(t) => t.label.len(),
        NodeKind::Button(b) => b.label.len(),
        NodeKind::ConversationPicker(p) | NodeKind::UserPicker(p) => p.label.len(),
        NodeKind::Callout(c) => opt(&c.title) + c.text.len(),
        NodeKind::Empty(e) => e.title.len() + opt(&e.text),
    };
    NODE_BYTES + opt(&n.id) + kind
}

/// A tree's size in bytes by the measure in rule 8 of the WIT `view`
/// interface.
pub fn size(tree: &ViewTree) -> usize {
    tree.nodes.iter().map(node_size).sum()
}

fn fail(rule: u8, why: String) -> Result<(), Error> {
    let why = format!("rule {rule}: {why}");
    Err(if rule == 8 {
        Error::Limit(why)
    } else {
        Error::Invalid(why)
    })
}

/// Checks a tree against the rules in the WIT `view` interface, in their
/// order, and reports the first broken one as `rule <n>: <why>`: `limit` for
/// rule 8, `invalid` for the rest. It does not know the plugin's grant, so
/// the image host check of rule 10 is the host's (and `FakeHost`'s).
pub fn validate(tree: &ViewTree) -> Result<(), Error> {
    let nodes = &tree.nodes;
    // 1. Root.
    if nodes.is_empty() {
        return fail(1, "an empty tree has no root node".into());
    }
    for (i, n) in nodes.iter().enumerate() {
        match (i, n.parent) {
            (0, Some(_)) => return fail(1, "node 0 is the root and has no parent".into()),
            (i, None) if i > 0 => {
                return fail(
                    1,
                    format!("node {i} has no parent; only node 0 is the root"),
                );
            }
            _ => {}
        }
    }
    // 2. Order.
    for (i, n) in nodes.iter().enumerate().skip(1) {
        let p = n.parent.unwrap_or(0);
        if p as usize >= i {
            return fail(
                2,
                format!("node {i} has parent {p}; a parent must come before its child"),
            );
        }
    }
    // 3. Ids.
    let mut ids = std::collections::HashSet::new();
    for n in nodes {
        if let Some(id) = &n.id
            && !ids.insert(id.as_str())
        {
            return fail(3, format!("node ids must be unique: `{id}` repeats"));
        }
    }
    // 4. Interactive.
    for (i, n) in nodes.iter().enumerate() {
        if n.id.is_none() && needs_id(&n.kind) {
            return fail(4, format!("node {i} ({}) needs an id", kind_name(&n.kind)));
        }
    }
    // 5. Leaves.
    let mut children = vec![0usize; nodes.len()];
    for n in nodes.iter().skip(1) {
        let p = n.parent.unwrap_or(0) as usize;
        if !is_container(&nodes[p].kind) {
            return fail(
                5,
                format!(
                    "node {p} ({}) cannot have children",
                    kind_name(&nodes[p].kind)
                ),
            );
        }
        children[p] += 1;
    }
    // 6. Tabs. 7. Split.
    for (i, n) in nodes.iter().enumerate() {
        if let NodeKind::Tabs(labels) = &n.kind
            && children[i] != labels.len()
        {
            return fail(
                6,
                format!(
                    "tabs node {i} has {} children for {} labels",
                    children[i],
                    labels.len()
                ),
            );
        }
    }
    for (i, n) in nodes.iter().enumerate() {
        if matches!(n.kind, NodeKind::Split) && children[i] != 2 {
            return fail(
                7,
                format!("split node {i} has {} children, needs 2", children[i]),
            );
        }
    }
    // 8. Size.
    if nodes.len() > MAX_NODES {
        return fail(
            8,
            format!("view tree has {} nodes, over {MAX_NODES}", nodes.len()),
        );
    }
    let bytes = size(tree);
    if bytes > MAX_BYTES {
        return fail(8, format!("view tree is {bytes} bytes, over {MAX_BYTES}"));
    }
    // 9. Labels.
    for (i, n) in nodes.iter().enumerate() {
        if accessible_label(&n.kind).is_some_and(|l| l.trim().is_empty()) {
            return fail(
                9,
                format!("node {i} ({}) needs a label", kind_name(&n.kind)),
            );
        }
    }
    // 10. Images.
    for (i, n) in nodes.iter().enumerate() {
        if let NodeKind::Image(img) = &n.kind
            && !image_source_ok(&img.src)
        {
            return fail(
                10,
                format!(
                    "node {i} image `{}` is not an assets/ path or https URL",
                    img.src
                ),
            );
        }
    }
    Ok(())
}

/// `ui.patch`: replaces the subtree rooted at the node with id `node` with
/// `patch`, whose node 0 takes its place, then checks the whole result
/// (so ids stay unique across the view).
pub fn apply_patch(base: &ViewTree, node: &str, patch: &ViewTree) -> Result<ViewTree, Error> {
    validate(patch)?;
    let Some(target) = base
        .nodes
        .iter()
        .position(|n| n.id.as_deref() == Some(node))
    else {
        return Err(Error::NotFound);
    };
    let mut out = Builder::unflatten(base);
    let replacement = Builder::unflatten(patch);
    out.replace(&mut 0, target, &mut Some(replacement));
    let tree = out.build();
    validate(&tree)?;
    Ok(tree)
}

impl Builder {
    /// The nested form of a flat tree (whose parents come first).
    fn unflatten(tree: &ViewTree) -> Builder {
        let mut items: Vec<Option<Builder>> = tree
            .nodes
            .iter()
            .map(|n| {
                Some(Builder {
                    id: n.id.clone(),
                    kind: n.kind.clone(),
                    children: vec![],
                })
            })
            .collect();
        for i in (1..items.len()).rev() {
            let child = items[i].take().unwrap();
            let p = tree.nodes[i].parent.unwrap_or(0) as usize;
            items[p].as_mut().unwrap().children.insert(0, child);
        }
        items[0].take().unwrap()
    }

    /// Replaces the node at document index `target`; `next` counts nodes.
    fn replace(&mut self, next: &mut usize, target: usize, with: &mut Option<Builder>) {
        if *next == target {
            *self = with.take().unwrap();
            return;
        }
        *next += 1;
        for c in &mut self.children {
            if with.is_none() {
                return;
            }
            c.replace(next, target, with);
        }
    }
}
