//! Strict, bounded schema-v2 input. No application code is executed by this crate.
mod source;
use schemars::JsonSchema;
use serde::{
    Deserialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
pub use source::{SourceMap, SourceSpan};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fmt,
    rc::Rc,
};

pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_DEPTH: usize = 64;
pub const MAX_NODES: usize = 16_384;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Document {
    #[serde(skip)]
    pub source: SourceMap,
    pub schema_version: u32,
    #[serde(default)]
    pub templates: BTreeMap<String, Template>,
    pub root: Control,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub root: Control,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Property {
    Literal(Literal),
    Bind(Bound),
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Literal {
    pub literal: Value,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bound {
    pub bind: Binding,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub path: String,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub source: BindingSource,
    pub converter: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, JsonSchema)]
pub enum BindingSource {
    #[default]
    Store,
    External,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, JsonSchema)]
pub enum Mode {
    #[default]
    OneWay,
    TwoWay,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
pub enum Orientation {
    #[default]
    Vertical,
    Horizontal,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Edges {
    Scalar(f32),
    Sides(Sides),
}
impl Default for Edges {
    fn default() -> Self {
        Self::Scalar(0.0)
    }
}
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Sides {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Placement {
    pub row: i32,
    pub column: i32,
    pub row_span: i32,
    pub column_span: i32,
    pub x: f32,
    pub y: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            row: 0,
            column: 0,
            row_span: 1,
            column_span: 1,
            x: 0.0,
            y: 0.0,
            width: None,
            height: None,
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Style {
    pub margin: Edges,
    pub padding: Edges,
    pub placement: Placement,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Validation {
    pub validator: String,
    pub error: String,
    pub pending: String,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Menu {
    pub id: String,
    pub title: Property,
    pub items: MenuItems,
    pub command: String,
    #[serde(default)]
    pub nested: bool,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MenuItems {
    Entries(Vec<MenuEntry>),
    Binding(Property),
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MenuEntry {
    pub id: String,
    pub label: Property,
    pub argument: Property,
    pub shortcut: Option<String>,
    #[serde(default)]
    pub role: MenuRole,
    pub enabled: Option<Property>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
pub enum MenuRole {
    #[default]
    Normal,
    Copy,
    Cut,
    Paste,
    SelectAll,
    About,
    Quit,
    Separator,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabPage {
    pub id: String,
    pub title: Property,
    pub content: Box<Control>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum TabPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

macro_rules! controls {
	($($kind:ident { $($fields:tt)* }),* $(,)?) => {
		#[derive(Clone, Debug, Deserialize, JsonSchema)]
		#[serde(tag = "type", deny_unknown_fields)]
		pub enum Control { $( $kind {
			id: String,
			#[serde(default)] style: Style,
			margin: Option<Edges>, padding: Option<Edges>,
            visible: Option<Property>, enabled: Option<Property>, style_binding: Option<Property>,
			$($fields)*
		}),* }
		impl Control {
			pub fn id(&self) -> &str { match self { $(Self::$kind { id, .. } => id),* } }
            pub fn appearance(&self) -> [(&'static str, Option<&Property>); 3] {
                match self { $(Self::$kind { visible, enabled, style_binding, .. } => [
                    ("visible", visible.as_ref()), ("enabled", enabled.as_ref()), ("style_binding", style_binding.as_ref())
                ]),* }
            }
			pub fn style(&self) -> Style { match self { $(Self::$kind {style,margin,padding,..} => {
				let mut s = style.clone();
				if let Some(v) = margin { s.margin = v.clone(); }
				if let Some(v) = padding { s.padding = v.clone(); }
				s
			}),* } }
		}
	}
}
controls! {
    Window { title: Property, content: Box<Control>, #[serde(default)] menus: Vec<Menu>, },
    Scroll { #[serde(default)] orientation: Orientation, content: Box<Control>, },
    StackLayout { #[serde(default)] orientation: Orientation, #[serde(default)] gap: f32, #[serde(default)] children: Vec<Control>, },
    Grid { columns: u32, #[serde(default)] gap: f32, #[serde(default)] children: Vec<Control>, },
    AbsoluteLayout { #[serde(default)] children: Vec<Control>, },
    FlexLayout { #[serde(default)] gap: f32, #[serde(default)] children: Vec<Control>, },
    BindableLayout { items: Property, item_template: String, viewport: Property, },
    ItemsControl { items: Property, item_template: String, viewport: Property, },
    ContentControl { content: Property, content_template: String, },
    Label { text: Property, },
    TextBox { text: Property, validation: Option<Validation>, },
    Button { text: Property, command: String, argument: Option<Property>, },
    Native { factory: String, value: Property, },
    CheckBox { text: Property, checked: Property, },
    Radio { text: Property, selected: Property, value: String, },
    Slider { value: Property, min: f64, max: f64, step: f64, },
    Combo { items: Property, selected: Property, },
    Image { source: Property, },
    Tab { selected: Property, #[serde(default)] position: TabPosition, pages: Vec<TabPage>, },
    Dialog { title: Property, open: Property, content: Box<Control>, },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub filename: String,
    pub path: String,
    pub message: String,
    pub span: Option<SourceSpan>,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = self.span {
            write!(
                f,
                "{}:{}:{}: {}: {}",
                self.filename, span.line, span.column, self.path, self.message
            )
        } else {
            write!(f, "{}:{}: {}", self.filename, self.path, self.message)
        }
    }
}
impl std::error::Error for Error {}

/// Parse JSON (`json`) or YAML (`yaml`/`yml`). Parser syntax diagnostics retain line/column.
/// Structural diagnostics include the filename and the closest serde/validation path.
pub fn parse(source: &str, format: &str, filename: &str) -> Result<Document, Error> {
    let error = |path: &str, message: String| Error {
        filename: filename.into(),
        path: path.into(),
        message,
        span: None,
    };
    if source.len() > MAX_BYTES {
        return Err(error("$", "input exceeds 1 MiB".into()));
    }
    let value = match format {
        "json" => {
            serde_json::from_str::<StrictValue>(source)
                .map_err(|e| error("$", e.to_string()))?
                .0
        }
        "yaml" | "yml" => {
            lexical_yaml(source).map_err(|e| error("$", e))?;
            let options = serde_saphyr::options! {
                strict_booleans: true,
                merge_keys: serde_saphyr::MergeKeyPolicy::AsOrdinary,
                reject_unsupported_tags: true,
            };
            serde_saphyr::from_str_with_options::<StrictValue>(source, options)
                .map_err(|e| error("$", e.to_string()))?
                .0
        }
        _ => return Err(error("$", "expected json, yaml or yml".into())),
    };
    let locations = SourceMap::parse(source, filename).map_err(|message| error("$", message))?;
    let diagnostic = |path: &str, message: String| Error {
        span: locations.diagnostic_span(path, &message),
        ..error(path, message)
    };
    let mut document: Document = serde_path_to_error::deserialize(value)
        .map_err(|e| diagnostic(&e.path().to_string(), e.inner().to_string()))?;
    document.validate().map_err(|e| diagnostic(&e.0, e.1))?;
    document.source = locations;
    Ok(document)
}

/// Stable FNV-1a fingerprint of exact source bytes (not a cryptographic digest).
pub fn fingerprint(source: &str) -> String {
    let hash = source.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}
/// Exportable JSON Schema, for schema-aware tooling; this is not an editor/LSP server.
pub fn schema() -> Value {
    serde_json::to_value(schemars::schema_for!(Document)).expect("JSON schema")
}

// Deserialize through a bounded visitor *before* allocating a recursive typed AST.
struct StrictValue(Value);
struct Seed {
    depth: usize,
    remaining: Rc<Cell<usize>>,
}
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Seed {
            depth: 0,
            remaining: Rc::new(Cell::new(MAX_NODES)),
        }
        .deserialize(d)
        .map(Self)
    }
}
impl<'de> DeserializeSeed<'de> for Seed {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        if self.depth > MAX_DEPTH {
            return Err(de::Error::custom("maximum depth 64 exceeded"));
        }
        let n = self.remaining.get();
        if n == 0 {
            return Err(de::Error::custom("node budget exceeded"));
        }
        self.remaining.set(n - 1);
        d.deserialize_any(self)
    }
}
impl Seed {
    fn child(&self) -> Self {
        Self {
            depth: self.depth + 1,
            remaining: self.remaining.clone(),
        }
    }
}
impl<'de> Visitor<'de> for Seed {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded JSON/YAML value")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite number"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = a.next_element_seed(self.child())? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut out = serde_json::Map::new();
        while let Some(k) = a.next_key::<String>()? {
            if out.contains_key(&k) {
                return Err(de::Error::custom(format!("duplicate key `{k}`")));
            }
            out.insert(k, a.next_value_seed(self.child())?);
        }
        Ok(Value::Object(out))
    }
}

// YAML tokens are inspected only outside quoted literals, comments and block scalars.
fn lexical_yaml(source: &str) -> Result<(), String> {
    use serde_saphyr::granit_parser::{ScalarStyle, Scanner, StrInput, TokenType};
    let scanner = Scanner::with_options(
        StrInput::new(source),
        serde_saphyr::granit_parser::options! { emit_comments: false },
    );
    let mut key = false;
    let mut depth = 0usize;
    for (index, token) in scanner.enumerate() {
        let token = token.map_err(|e| e.to_string())?;
        let mark = token.span().start;
        let fail = |message| format!("line {} column {}: {message}", mark.line(), mark.col() + 1);
        if index > MAX_NODES * 8 {
            return Err(fail("lexical node budget exceeded"));
        }
        match token.token_type() {
            TokenType::Alias(_)
            | TokenType::Anchor(_)
            | TokenType::Tag(..)
            | TokenType::TagDirective(..) => {
                return Err(fail("aliases, anchors and tags are forbidden"));
            }
            TokenType::Scalar(ScalarStyle::Plain, text) if key && text == "<<" => {
                return Err(fail("merge keys are forbidden"));
            }
            TokenType::BlockMappingStart
            | TokenType::BlockSequenceStart
            | TokenType::FlowMappingStart
            | TokenType::FlowSequenceStart => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(fail("maximum depth 64 exceeded"));
                }
            }
            TokenType::BlockEnd | TokenType::FlowMappingEnd | TokenType::FlowSequenceEnd => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        key = matches!(token.token_type(), TokenType::Key);
    }
    Ok(())
}

impl Document {
    pub fn validate(&self) -> Result<(), (String, String)> {
        if self.schema_version != 2 {
            return Err(("schema_version".into(), "expected schema version 2".into()));
        }
        if !matches!(self.root, Control::Window { .. }) {
            return Err(("root".into(), "root must be Window".into()));
        }
        let mut work = 0;
        self.walk(
            &self.root,
            "root",
            &mut BTreeSet::new(),
            &mut Vec::new(),
            &mut work,
            0,
        )?;
        // Validate unused templates too, including indirect cycles.
        for (name, t) in &self.templates {
            self.walk(
                &t.root,
                &format!("templates.{name}.root"),
                &mut BTreeSet::new(),
                &mut vec![name.clone()],
                &mut work,
                0,
            )?;
        }
        Ok(())
    }
    fn walk(
        &self,
        c: &Control,
        path: &str,
        ids: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
        work: &mut usize,
        depth: usize,
    ) -> Result<(), (String, String)> {
        let err = |s: String| (path.to_string(), s);
        *work += 1;
        if *work > MAX_NODES {
            return Err(err("expanded template node budget exceeded".into()));
        }
        if depth > MAX_DEPTH {
            return Err(err("expanded control/template depth 64 exceeded".into()));
        }
        if c.id().is_empty() || c.id().contains('/') {
            return Err(err("id must be nonempty and contain no `/`".into()));
        }
        if c.id() == "__root" {
            return Err(err("id `__root` is reserved for the backend root".into()));
        }
        if !ids.insert(c.id().into()) {
            return Err(err(format!("duplicate id `{}`", c.id())));
        }
        let s = c.style();
        let sides = |e: &Edges| match e {
            Edges::Scalar(v) => vec![*v; 4],
            Edges::Sides(v) => vec![v.top, v.right, v.bottom, v.left],
        };
        let p = s.placement;
        if sides(&s.margin)
            .iter()
            .chain(sides(&s.padding).iter())
            .any(|x| !x.is_finite() || !(0.0..=panechord_runtime::MAX_LAYOUT_SIZE).contains(x))
            || p.row < 0
            || p.column < 0
            || p.row_span < 1
            || p.column_span < 1
            || !p.x.is_finite()
            || !p.y.is_finite()
            || p.x.abs() > panechord_runtime::MAX_LAYOUT_SIZE
            || p.y.abs() > panechord_runtime::MAX_LAYOUT_SIZE
            || [p.width, p.height]
                .into_iter()
                .flatten()
                .any(|v| !v.is_finite() || v < 0.0 || v > 1_000_000.0)
        {
            return Err(err("invalid spacing or placement".into()));
        }
        if sides(&s.margin)
            .iter()
            .zip(sides(&s.padding))
            .any(|(margin, padding)| margin + padding > panechord_runtime::MAX_LAYOUT_SIZE)
        {
            return Err(err("combined margin and padding exceed layout limit".into()));
        }
        let read_only = |property: &Property, name: &str| {
            if matches!(property, Property::Bind(bound) if bound.bind.mode == Mode::TwoWay) {
                Err((
                    format!("{path}.{name}.bind.mode"),
                    "TwoWay is not supported for a read-only property".into(),
                ))
            } else {
                Ok(())
            }
        };
        for (name, property) in c.appearance() {
            if let Some(property) = property {
                read_only(property, name)?;
                if name != "style_binding"
                    && matches!(property, Property::Literal(value) if !value.literal.is_boolean())
                {
                    return Err((format!("{path}.{name}"), "expected boolean literal".into()));
                }
            }
        }
        match c {
            Control::Label { text, .. }
            | Control::CheckBox { text, .. }
            | Control::Radio { text, .. } => read_only(text, "text")?,
            Control::Button { text, argument, .. } => {
                read_only(text, "text")?;
                if let Some(argument) = argument {
                    read_only(argument, "argument")?;
                }
            }
            Control::Image { source, .. } => read_only(source, "source")?,
            Control::Combo { items, .. }
            | Control::ItemsControl { items, .. }
            | Control::BindableLayout { items, .. } => read_only(items, "items")?,
            Control::ContentControl { content, .. } => read_only(content, "content")?,
            Control::Window { title, .. } | Control::Dialog { title, .. } => {
                read_only(title, "title")?
            }
            _ => {}
        }
        let mut children = Vec::new();
        let template = match c {
            Control::Window { content, menus, .. } => {
                if depth != 0 || path != "root" {
                    return Err(err(
                        "nested Window is unsupported by the single-View API".into()
                    ));
                }
                for (index, m) in menus.iter().enumerate() {
                    read_only(&m.title, &format!("menus[{index}].title"))?;
                    match &m.items {
                        MenuItems::Binding(items) => {
                            read_only(items, &format!("menus[{index}].items"))?
                        }
                        MenuItems::Entries(entries) => {
                            if m.nested {
                                return Err(err("static entries cannot use nested menus".into()));
                            }
                            for (entry_index, entry) in entries.iter().enumerate() {
                                let entry_path = format!("menus[{index}].items[{entry_index}]");
                                read_only(&entry.label, &format!("{entry_path}.label"))?;
                                read_only(&entry.argument, &format!("{entry_path}.argument"))?;
                                if let Some(enabled) = &entry.enabled {
                                    read_only(enabled, &format!("{entry_path}.enabled"))?;
                                }
                                if entry.id.is_empty()
                                    || entry.id.contains('/')
                                    || !ids.insert(entry.id.clone())
                                {
                                    return Err(err(format!(
                                        "invalid or duplicate menu item id `{}`",
                                        entry.id
                                    )));
                                }
                            }
                        }
                    }
                    if m.id == "__root" {
                        return Err(err(
                            "menu id `__root` is reserved for the backend root".into()
                        ));
                    }
                    if m.id.is_empty() || m.id.contains('/') || !ids.insert(m.id.clone()) {
                        return Err(err(format!("invalid or duplicate menu id `{}`", m.id)));
                    }
                }
                self.walk(
                    content,
                    &format!("{path}.content"),
                    ids,
                    stack,
                    work,
                    depth + 1,
                )?;
                None
            }
            Control::Grid {
                columns,
                gap,
                children: cs,
                ..
            } => {
                if !(1..=4096).contains(columns)
                    || !gap.is_finite()
                    || !(0.0..=1_000_000.0).contains(gap)
                {
                    return Err(err("invalid grid columns/gap".into()));
                }
                children.extend(cs);
                None
            }
            Control::StackLayout {
                gap, children: cs, ..
            }
            | Control::FlexLayout {
                gap, children: cs, ..
            } => {
                if !gap.is_finite() || !(0.0..=1_000_000.0).contains(gap) {
                    return Err(err("invalid gap".into()));
                }
                children.extend(cs);
                None
            }
            Control::AbsoluteLayout { children: cs, .. } => {
                children.extend(cs);
                None
            }
            Control::Dialog { content, .. } | Control::Scroll { content, .. } => {
                self.walk(
                    content,
                    &format!("{path}.content"),
                    ids,
                    stack,
                    work,
                    depth + 1,
                )?;
                None
            }
            Control::Tab {
                selected, pages, ..
            } => {
                if pages.is_empty() || pages.len() > 256 {
                    return Err(err("Tab requires 1..=256 pages".into()));
                }
                if let Property::Literal(value) = selected {
                    let selected = value.literal.as_str().ok_or_else(|| {
                        (
                            format!("{path}.selected"),
                            "Tab selection must be a string".into(),
                        )
                    })?;
                    if !selected.is_empty() && !pages.iter().any(|page| page.id == selected) {
                        return Err((
                            format!("{path}.selected"),
                            "unknown selected tab page".into(),
                        ));
                    }
                }
                for (index, page) in pages.iter().enumerate() {
                    if !panechord_runtime::valid_item_key(&page.id)
                        || page.id == "__root"
                        || !ids.insert(page.id.clone())
                    {
                        return Err(err("invalid or duplicate tab page id".into()));
                    }
                    read_only(&page.title, &format!("pages[{index}].title"))?;
                    self.walk(
                        &page.content,
                        &format!("{path}.pages[{index}].content"),
                        ids,
                        stack,
                        work,
                        depth + 1,
                    )?;
                }
                None
            }
            Control::Slider { min, max, step, .. } => {
                panechord_runtime::SliderRange {
                    min: *min,
                    max: *max,
                    step: *step,
                }
                .validate()
                .map_err(|message| err(message.into()))?;
                None
            }
            Control::Radio { value, .. } => {
                if !panechord_runtime::valid_item_key(value) {
                    return Err(err("invalid radio value".into()));
                }
                None
            }
            Control::ItemsControl { item_template, .. }
            | Control::BindableLayout { item_template, .. } => Some(item_template),
            Control::ContentControl {
                content_template, ..
            } => Some(content_template),
            _ => None,
        };
        for (index, child) in children.into_iter().enumerate() {
            self.walk(
                child,
                &format!("{path}.children[{index}]"),
                ids,
                stack,
                work,
                depth + 1,
            )?;
        }
        if let Some(name) = template {
            if stack.contains(name) {
                return Err(err(format!(
                    "recursive template: {} -> {name}",
                    stack.join(" -> ")
                )));
            }
            let t = self
                .templates
                .get(name)
                .ok_or_else(|| err(format!("unknown template `{name}`")))?;
            stack.push(name.clone());
            self.walk(
                &t.root,
                &format!("templates.{name}.root"),
                &mut BTreeSet::new(),
                stack,
                work,
                depth + 1,
            )?;
            stack.pop();
        }
        Ok(())
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    #[test]
    fn scroll_content_is_validated_with_normal_control_bounds() {
        let source = r#"{"schema_version":2,"root":{"type":"Window","id":"root_window","title":{"literal":"Test"},"content":{"type":"Scroll","id":"viewport","content":{"type":"Label","id":"label","text":{"literal":"Content"}}}}}"#;
        parse(source, "json", "scroll.json").unwrap();
        let duplicate = source.replace("\"id\":\"label\"", "\"id\":\"viewport\"");
        assert!(parse(&duplicate, "json", "scroll.json").is_err());
        let nested = source.replace("\"type\":\"Label\",\"id\":\"label\",\"text\":{\"literal\":\"Content\"}", "\"type\":\"Window\",\"id\":\"window\",\"title\":{\"literal\":\"Nested\"},\"content\":{\"type\":\"Label\",\"id\":\"label\",\"text\":{\"literal\":\"Content\"}}");
        assert!(parse(&nested, "json", "scroll.json").is_err());
    }
}
