//! Host-only validating compiler. Emits native construction calls, not DTOs.
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::{collections::BTreeSet, fmt, fs, path::Path};
use wakui_contract::{Descriptor, ValueType};
use wakui_definition::{Binding, Document, MenuNodeDef, Mode, Node, Property, Registry};

#[derive(Debug)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Gtk,
    Tao,
}
impl Backend {
    pub fn select(gtk: bool, tao: bool) -> Result<Self> {
        match (gtk, tao) {
            (true, false) => Ok(Self::Gtk),
            (false, true) => Ok(Self::Tao),
            _ => Err(Error(
                "select exactly one backend-gtk or backend-tao feature".into(),
            )),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Gtk => "gtk",
            Self::Tao => "tao",
        }
    }
}

/// JSON-compatible value with recursive duplicate-key rejection for BOTH formats.
struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Strict;
        impl<'de> Visitor<'de> for Strict {
            type Value = StrictValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON-compatible data")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(StrictValue(v)) = a.next_element()? {
                    out.push(v);
                }
                Ok(StrictValue(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some((k, StrictValue(v))) = a.next_entry::<String, StrictValue>()? {
                    if out.insert(k.clone(), v).is_some() {
                        return Err(de::Error::custom(format!("duplicate key: {k}")));
                    }
                }
                Ok(StrictValue(Value::Object(out)))
            }
        }
        d.deserialize_any(Strict)
    }
}

pub fn parse(source: &str, format: &str, registry: &Registry) -> Result<Document> {
    if source.len() > 1024 * 1024 {
        return Err(Error("UI document exceeds 1 MiB".into()));
    }
    let StrictValue(value) = match format {
        "json" => serde_json::from_str(source).map_err(|e| Error(e.to_string()))?,
        "yaml" | "yml" => serde_saphyr::from_str(source).map_err(|e| Error(e.to_string()))?,
        _ => return Err(Error(format!("unsupported UI format: {format}"))),
    };
    fn check(value: &Value, registry: &Registry, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(Error("UI nesting exceeds 64 levels".into()));
        }
        match value {
            Value::Object(map) => {
                if let Some(tag) = map.get("type").and_then(Value::as_str)
                    && !registry.contains(tag)
                {
                    return Err(Error(format!("unregistered Control/Menu type: {tag}")));
                }
                for v in map.values() {
                    check(v, registry, depth + 1)?;
                }
            }
            Value::Array(values) => {
                for v in values {
                    check(v, registry, depth + 1)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
    check(&value, registry, 0)?;
    serde_json::from_value(value).map_err(|e| Error(e.to_string()))
}

pub struct Compiler<'a> {
    contract: &'a Descriptor,
    document: &'a Document,
    ids: BTreeSet<String>,
    source: String,
    counter: usize,
}

impl<'a> Compiler<'a> {
    pub fn generate(
        document: &'a Document,
        contract: &'a Descriptor,
        backend: Backend,
        contract_rust: &str,
    ) -> Result<String> {
        if document.schema_version != 1 {
            return Err(Error("unsupported schema_version (expected 1)".into()));
        }
        if document.view_model != contract.name {
            return Err(Error(format!("unknown ViewModel: {}", document.view_model)));
        }
        let mut compiler = Self {
            contract,
            document,
            ids: BTreeSet::new(),
            source: String::new(),
            counter: 0,
        };
        let root = document.root.lower().map_err(|e| Error(e.to_string()))?;
        let Node::Window {
            id,
            title,
            content,
            menu,
        } = root
        else {
            return Err(Error("root must be a Window".into()));
        };
        compiler.id(&id)?;
        let title = compiler.text(&title, &id, false)?;
        compiler.source = format!(
            "// Generated by wakui-build. Do not edit.\nuse wakui_{} as backend;\npub const BACKEND: &str = {:?};\npub fn create<M: wakui_runtime::ViewModel<Contract = {contract_rust}>>(model: &wakui_runtime::ModelHandle<M>) -> backend::View {{\nlet view = backend::View::new({title});\nlet root = view.root();\n",
            backend.name(),
            backend.name()
        );
        compiler.node(&content, "root")?;
        for entry in &menu {
            compiler.menu(entry)?;
        }
        compiler.source.push_str("view\n}\n");
        Ok(compiler.source)
    }

    fn id(&mut self, id: &str) -> Result<()> {
        if id.is_empty() || !self.ids.insert(id.into()) {
            return Err(Error(format!("duplicate or empty node id: {id:?}")));
        }
        Ok(())
    }

    fn property(
        &self,
        binding: &Binding,
        ty: ValueType,
        id: &str,
        editable: bool,
    ) -> Result<String> {
        let p = self
            .contract
            .properties
            .iter()
            .find(|p| p.name == binding.path)
            .ok_or_else(|| Error(format!("{id}: unknown Binding path {:?}", binding.path)))?;
        if p.value_type != ty {
            return Err(Error(format!(
                "{id}.{}: expected {ty:?}, got {:?}",
                p.name, p.value_type
            )));
        }
        if binding.mode == Mode::TwoWay && (!editable || !p.writable) {
            return Err(Error(format!(
                "{id}.{}: TwoWay requires an editable attribute and writable property",
                p.name
            )));
        }
        let value = format!("model.bind(&{})", p.rust_key);
        // A writable model property MUST NOT make a OneWay native entry writable.
        Ok(if binding.mode == Mode::OneWay {
            format!("{value}.map(|value| value)")
        } else {
            value
        })
    }

    fn text(&self, p: &Property<String>, id: &str, editable: bool) -> Result<String> {
        match p {
            Property::Literal(s) => Ok(format!(
                "wakui_runtime::Binding::constant({s:?}.to_owned())"
            )),
            Property::Bind(b) => self.property(b, ValueType::Text, id, editable),
            Property::Resource(key) => {
                let s = self
                    .document
                    .resources
                    .get(key)
                    .ok_or_else(|| Error(format!("{id}: unknown resource {key}")))?;
                Ok(format!(
                    "wakui_runtime::Binding::constant({s:?}.to_owned())"
                ))
            }
        }
    }

    fn command(&self, command: &wakui_definition::CommandRef, id: &str) -> Result<String> {
        let command = self
            .contract
            .commands
            .iter()
            .find(|c| c.name == command.path)
            .ok_or_else(|| Error(format!("{id}: unknown Command {}", command.path)))?;
        Ok(format!("model.command(&{})", command.rust_key))
    }

    fn node(&mut self, node: &Node, parent: &str) -> Result<()> {
        match node {
            Node::Window { .. } => return Err(Error("Window cannot be a child control".into())),
            Node::Panel { id, gap, children } => {
                self.id(id)?;
                if *gap > 128 {
                    return Err(Error(format!("{id}: panel gap exceeds 128 DIP")));
                }
                let name = format!("panel{}", self.counter);
                self.counter += 1;
                self.source.push_str(&format!(
                    "let {name} = view.panel(&{parent}, {id:?}, {gap});\n"
                ));
                for child in children {
                    self.node(child, &name)?;
                }
            }
            Node::Label { id, text } | Node::Entry { id, text } => {
                self.id(id)?;
                let entry = matches!(node, Node::Entry { .. });
                let text = self.text(text, id, entry)?;
                let method = if entry { "entry" } else { "label" };
                self.source
                    .push_str(&format!("view.{method}(&{parent}, {id:?}, {text});\n"));
            }
            Node::Button { id, text, command } => {
                self.id(id)?;
                let text = self.text(text, id, false)?;
                let command = self.command(command, id)?;
                self.source.push_str(&format!(
                    "view.button(&{parent}, {id:?}, {text}, {command});\n"
                ));
            }
            Node::Items {
                id,
                items,
                item_key,
                template,
            } => {
                self.id(id)?;
                let p = self
                    .contract
                    .properties
                    .iter()
                    .find(|p| p.name == items.path)
                    .ok_or_else(|| Error(format!("{id}: unknown items path {}", items.path)))?;
                let ValueType::Items {
                    contract,
                    key,
                    text,
                } = &p.value_type
                else {
                    return Err(Error(format!(
                        "{id}: expected Items, got {:?}",
                        p.value_type
                    )));
                };
                if items.mode != Mode::OneWay || item_key != key {
                    return Err(Error(format!("{id}: invalid item key/mode")));
                }
                let template = self
                    .document
                    .templates
                    .get(template)
                    .ok_or_else(|| Error(format!("{id}: unknown template {template}")))?;
                if template.data_type != *contract {
                    return Err(Error(format!("{id}: wrong template data_type")));
                }
                let row = template.root.lower().map_err(|e| Error(e.to_string()))?;
                let Node::Label {
                    text: Property::Bind(b),
                    ..
                } = row
                else {
                    return Err(Error(format!(
                        "{id}: prototype supports Label row templates only"
                    )));
                };
                if b.path != *text || b.mode != Mode::OneWay {
                    return Err(Error(format!("{id}: invalid row binding {}", b.path)));
                }
                self.source.push_str(&format!(
                    "view.items(&{parent}, {id:?}, model.bind(&{}), |item| item.text.clone());\n",
                    p.rust_key
                ));
            }
        }
        Ok(())
    }

    fn menu(&mut self, entry: &MenuNodeDef) -> Result<()> {
        let MenuNodeDef::Submenu { id, text, items } = entry else {
            return Err(Error("menubar requires Submenu".into()));
        };
        self.id(id)?;
        let name = format!("menu{}", self.counter);
        self.counter += 1;
        self.source
            .push_str(&format!("let {name} = view.menu({id:?}, {text:?});\n"));
        for entry in items {
            let MenuNodeDef::Item { id, text, command } = entry else {
                return Err(Error("prototype supports one submenu level".into()));
            };
            self.id(id)?;
            let text = self.text(text, id, false)?;
            let command = self.command(command, id)?;
            self.source.push_str(&format!(
                "view.menu_item(&{name}, {id:?}, {text}, {command});\n"
            ));
        }
        Ok(())
    }
}

pub fn compile_file(
    path: &Path,
    out: &Path,
    registry: &Registry,
    contract: &Descriptor,
    backend: Backend,
    contract_rust: &str,
    target: &str,
) -> Result<()> {
    println!("cargo:rerun-if-changed={}", path.display());
    let result = (|| {
        let source = fs::read_to_string(path).map_err(|e| Error(e.to_string()))?;
        let format = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let document = parse(&source, format, registry)?;
        let generated = Compiler::generate(&document, contract, backend, contract_rust)?;
        fs::write(out.join("view_generated.rs"), generated).map_err(|e| Error(e.to_string()))?;
        let manifest = serde_json::json!({"schema_version":1,"generated_abi":1,"backend":backend.name(),"target":target,"contract":contract.name});
        fs::write(
            out.join("view_manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .map_err(|e| Error(e.to_string()))
    })();
    result.map_err(|e| Error(format!("{}: {e}", path.display())))
}
