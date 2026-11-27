//! JSON edits splice only the named member. Third-party numbers, escaping and whitespace survive.
use envlume_application::integrations::{Fault, Launch};
use envlume_domain::ClientKind;
use serde::{
    Deserialize, Deserializer,
    de::{Error, MapAccess, Visitor},
};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::fmt;
use toml_edit::{Array, DocumentMut, Item, Table};

pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn signature(launch: &Launch) -> String {
    hash(&serde_json::to_vec(launch).expect("strings serialize"))
}
pub(crate) enum Edit<'a> {
    Inspect,
    Add(&'a Launch),
    Remove,
}
pub(crate) struct Document {
    pub entry: Option<String>,
    pub bytes: Vec<u8>,
}

struct Members<'a>(Vec<(String, &'a RawValue)>);
impl<'de> Deserialize<'de> for Members<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Object;
        impl<'de> Visitor<'de> for Object {
            type Value = Members<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object with unique keys")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut entries = Vec::new();
                let mut seen = std::collections::HashSet::new();
                while let Some((key, value)) = map.next_entry::<String, &'de RawValue>()? {
                    if !seen.insert(key.clone()) {
                        return Err(M::Error::custom("duplicate key"));
                    }
                    entries.push((key, value));
                }
                Ok(Members(entries))
            }
        }
        d.deserialize_map(Object)
    }
}
fn members(raw: &str) -> Result<Members<'_>, Fault> {
    serde_json::from_str(raw).map_err(|_| Fault::InvalidConfig)
}
fn json(source: &str, edit: Edit<'_>) -> Result<Document, Fault> {
    let root = members(source)?;
    let namespace = root
        .0
        .iter()
        .find(|(k, _)| k == "mcpServers")
        .map(|(_, v)| v.get());
    let object = namespace.unwrap_or(source.trim_end());
    let local = members(object)?;
    let at = object.as_ptr() as usize - source.as_ptr() as usize;
    let start = |v: &RawValue| v.get().as_ptr() as usize - source.as_ptr() as usize;
    let idx = namespace.and_then(|_| local.0.iter().position(|(k, _)| k == "envlume"));
    let entry = idx.map(|i| {
        serde_json::from_str::<Launch>(local.0[i].1.get())
            .map(|l| signature(&l))
            .unwrap_or_else(|_| "foreign".into())
    });
    let mut out = source.to_owned();
    match edit {
        Edit::Add(l) if idx.is_none() => {
            let value = serde_json::to_string(l).map_err(|_| Fault::InvalidConfig)?;
            let member = format!(r#""envlume":{value}"#);
            let field = if namespace.is_some() {
                member
            } else {
                format!(r#""mcpServers":{{{member}}}"#)
            };
            let comma = if local.0.is_empty() { "" } else { "," };
            out.insert_str(at + object.len() - 1, &format!("{comma}\n{field}\n"));
        }
        Edit::Remove if idx.is_some() => {
            let i = idx.unwrap();
            let current = local.0[i].1;
            let left = if i == 0 {
                at + object.find('{').unwrap() + 1
            } else {
                let prev = local.0[i - 1].1;
                start(prev) + prev.get().len()
            };
            let end = start(current) + current.get().len();
            let right = if i == 0 && local.0.len() > 1 {
                end + source[end..].find(',').unwrap() + 1
            } else {
                end
            };
            out.replace_range(left..right, "");
        }
        _ => (),
    }
    Ok(Document {
        entry,
        bytes: out.into_bytes(),
    })
}
fn toml(source: &str, edit: Edit<'_>) -> Result<Document, Fault> {
    let mut doc = source
        .parse::<DocumentMut>()
        .map_err(|_| Fault::InvalidConfig)?;
    let table = doc
        .get("mcp_servers")
        .map(|i| i.as_table_like().ok_or(Fault::InvalidConfig))
        .transpose()?;
    let entry = table.and_then(|t| t.get("envlume")).map(|item| {
        let parsed = (|| {
            let t = item.as_table_like()?;
            if t.len() != 2 {
                return None;
            }
            Some(Launch {
                command: t.get("command")?.as_str()?.into(),
                args: t
                    .get("args")?
                    .as_array()?
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned))
                    .collect::<Option<_>>()?,
            })
        })();
        parsed
            .as_ref()
            .map(signature)
            .unwrap_or_else(|| "foreign".into())
    });
    match edit {
        Edit::Add(l) if entry.is_none() => {
            if table.is_none() {
                doc.insert("mcp_servers", Item::Table(Table::new()));
            }
            let mut t = Table::new();
            t.insert("command", toml_edit::value(&l.command));
            let mut args = Array::new();
            for a in &l.args {
                args.push(a.as_str());
            }
            t.insert("args", toml_edit::value(args));
            doc["mcp_servers"]
                .as_table_like_mut()
                .unwrap()
                .insert("envlume", Item::Table(t));
        }
        Edit::Remove if entry.is_some() => {
            doc["mcp_servers"]
                .as_table_like_mut()
                .unwrap()
                .remove("envlume");
        }
        _ => {
            return Ok(Document {
                entry,
                bytes: source.as_bytes().into(),
            });
        }
    }
    Ok(Document {
        entry,
        bytes: doc.to_string().into_bytes(),
    })
}
pub(crate) fn document(
    client: ClientKind,
    bytes: Option<&[u8]>,
    edit: Edit<'_>,
) -> Result<Document, Fault> {
    let toml_format = client == ClientKind::Codex;
    let source = std::str::from_utf8(bytes.unwrap_or(if toml_format { b"" } else { b"{}\n" }))
        .map_err(|_| Fault::InvalidConfig)?;
    if toml_format {
        toml(source, edit)
    } else {
        json(source, edit)
    }
}
