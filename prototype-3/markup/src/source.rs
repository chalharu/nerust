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
    keys: BTreeMap<String, SourceSpan>,
    nodes: BTreeMap<String, SourceSpan>,
}
enum Node {
    Map(Vec<(Spanned<String>, Spanned<Node>)>),
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
                let mut values = Vec::new();
                while let Some((key, value)) = map.next_entry()? {
                    values.push((key, value));
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
                if let Some((_, id)) = values.iter().find(|(key, _)| key.value == "id")
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
                        key.value.clone()
                    } else {
                        format!("{path}.{}", key.value)
                    };
                    self.keys.insert(
                        next.clone(),
                        SourceSpan {
                            line: key.referenced.line(),
                            column: key.referenced.column(),
                        },
                    );
                    self.walk(
                        value,
                        &next,
                        if path == "templates" {
                            &key.value
                        } else {
                            scope
                        },
                    );
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
            if let Some(span) = self.keys.get(&format!("{path}.{field}")) {
                return Some(*span);
            }
            let prefix = format!("{path}.");
            let suffix = format!(".{field}");
            let mut candidates = self.keys.iter().filter(|(candidate, _)| {
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

#[test]
fn json_and_crlf_have_node_locations() {
    let map = SourceMap::parse(
        "{\r\n\t\"root\": {\"title\": \"日本語\", \"id\": \"name\"}\r\n}",
        "ui.json",
    )
    .unwrap();
    let span = map.node(None, "name").unwrap();
    assert_eq!(span.line, 2);
    assert!(span.column > 1);
    assert_eq!(map.closest("root.title").unwrap().line, 2);
}

#[test]
fn repeated_unknown_fields_resolve_to_their_own_key_not_the_parent() {
    let map = SourceMap::parse("root:\n  children:\n    - id: first\n      typo: true\n    - id: second\n      typo: false\n", "ui.yaml").unwrap();
    assert_eq!(
        map.diagnostic_span("root.children[0]", "unknown field `typo`"),
        Some(SourceSpan { line: 4, column: 7 })
    );
    assert_eq!(
        map.diagnostic_span("root.children[1]", "unknown field `typo`"),
        Some(SourceSpan { line: 6, column: 7 })
    );
    assert_eq!(
        map.diagnostic_span("root", "unknown field `typo`"),
        map.closest("root")
    );
}
