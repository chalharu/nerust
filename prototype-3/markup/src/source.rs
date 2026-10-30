use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_saphyr::Spanned;
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub line: u64,
    pub column: u64,
}
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    pub filename: String,
    paths: BTreeMap<String, SourceSpan>,
    nodes: BTreeMap<String, SourceSpan>,
}
enum Node {
    Map(BTreeMap<String, Spanned<Node>>),
    Seq(Vec<Spanned<Node>>),
    Text(String),
    Scalar,
}
impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NodeVisitor;
        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = Node;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a YAML value")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
                let mut values = BTreeMap::new();
                while let Some((key, value)) = map.next_entry()? {
                    values.insert(key, value);
                }
                Ok(Node::Map(values))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element()? {
                    values.push(value);
                }
                Ok(Node::Seq(values))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Node, E> {
                Ok(Node::Text(value.into()))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Node, E> {
                Ok(Node::Text(value))
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Node, E> {
                Ok(Node::Scalar)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Node, E> {
                Ok(Node::Scalar)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Node, E> {
                Ok(Node::Scalar)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Node, E> {
                Ok(Node::Scalar)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Node, E> {
                Ok(Node::Scalar)
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Node, E> {
                Ok(Node::Scalar)
            }
        }
        deserializer.deserialize_any(NodeVisitor)
    }
}
impl SourceMap {
    pub fn parse(source: &str, filename: &str) -> Result<Self, String> {
        let root: Spanned<Node> =
            serde_saphyr::from_str(source).map_err(|error| error.to_string())?;
        let mut map = Self {
            filename: filename.into(),
            ..Self::default()
        };
        map.walk(&root, "", "");
        Ok(map)
    }
    fn walk(&mut self, node: &Spanned<Node>, path: &str, scope: &str) {
        let location = node.referenced;
        self.paths.insert(
            path.into(),
            SourceSpan {
                line: location.line(),
                column: location.column(),
            },
        );
        match &node.value {
            Node::Map(values) => {
                if let Some(id) = values.get("id")
                    && let Node::Text(id_text) = &id.value
                {
                    self.nodes.insert(
                        format!("{scope}/{id_text}"),
                        SourceSpan {
                            line: id.referenced.line(),
                            column: id.referenced.column(),
                        },
                    );
                }
                for (key, value) in values {
                    let next = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    self.walk(value, &next, if path == "templates" { key } else { scope });
                }
            }
            Node::Seq(values) => {
                for (index, value) in values.iter().enumerate() {
                    self.walk(value, &format!("{path}[{index}]"), scope);
                }
            }
            _ => (),
        }
    }
    pub fn node(&self, scope: Option<&str>, id: &str) -> Option<SourceSpan> {
        self.nodes
            .get(&format!("{}/{id}", scope.unwrap_or("")))
            .copied()
    }
    pub fn closest(&self, path: &str) -> Option<SourceSpan> {
        let mut path = path;
        loop {
            if let Some(span) = self.paths.get(path) {
                return Some(*span);
            }
            let end = path.rfind(['.', '['])?;
            path = &path[..end];
        }
    }
    pub fn diagnostic_span(&self, path: &str, message: &str) -> Option<SourceSpan> {
        if let Some(tail) = message.strip_prefix("unknown field `")
            && let Some((field, _)) = tail.split_once('`')
        {
            let prefix = format!("{path}.");
            let suffix = format!(".{field}");
            let mut candidates = self.paths.iter().filter(|(candidate, _)| {
                candidate.starts_with(&prefix) && candidate.ends_with(&suffix)
            });
            if let Some((_, span)) = candidates.next()
                && candidates.next().is_none()
            {
                return Some(*span);
            }
        }
        self.closest(path)
    }
}

#[test]
fn maps_nodes_templates_unicode_and_structural_paths() {
    let map = SourceMap::parse("root:\n  id: name\n  children:\n    - id: child\ntemplates:\n  row:\n    root:\n      id: name\n", "ui.yaml").unwrap();
    assert_eq!(
        map.node(None, "name"),
        Some(SourceSpan { line: 2, column: 7 })
    );
    assert_eq!(map.node(Some("row"), "name").unwrap().line, 8);
    assert_eq!(map.closest("root.children[0].typo").unwrap().line, 4);
}
