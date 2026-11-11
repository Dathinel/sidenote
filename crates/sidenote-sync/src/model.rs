//! Field registers, observed-remove check tokens, and character operations.
//! Create each domain root once; joining devices must import that history.
use crate::status::{Failure, Reason};
use anyhow::{Context, Result, ensure};
use automerge::{Automerge, ObjId, ObjType, ROOT, ReadDoc, transaction::Transactable};
use rand::{RngCore, rngs::OsRng};

fn map(doc: &Automerge, parent: &ObjId, key: &str) -> Result<ObjId> {
    let (value, obj) = doc.get(parent, key)?.context("missing map")?;
    ensure!(
        value == automerge::Value::Object(ObjType::Map),
        "expected map"
    );
    Ok(obj)
}

pub fn object(doc: &Automerge, id: &str) -> Result<ObjId> {
    map(doc, &map(doc, &ROOT, "objects")?, id)
}

pub fn create(doc: &mut Automerge, id: &str, title: &str) -> Result<()> {
    ensure!(id.len() <= 128 && !id.is_empty(), "invalid object ID");
    let objects = doc.get(ROOT, "objects")?.map(|(_, o)| o);
    if let Some(ref objects) = objects {
        ensure!(doc.get(objects, id)?.is_none(), "object exists");
    }
    let mut tx = doc.transaction();
    let objects = match objects {
        Some(o) => o,
        None => tx.put_object(ROOT, "objects", ObjType::Map)?,
    };
    let obj = tx.put_object(objects, id, ObjType::Map)?;
    let text = tx.put_object(&obj, "title", ObjType::Text)?;
    tx.splice_text(text, 0, 0, title)?;
    tx.put_object(&obj, "fields", ObjType::Map)?;
    tx.put_object(&obj, "checked_tokens", ObjType::Map)?;
    tx.put_object(&obj, "deleted_tokens", ObjType::Map)?;
    tx.put_object(&obj, "recovery", ObjType::Map)?;
    tx.commit();
    Ok(())
}

pub fn put(doc: &mut Automerge, id: &str, field: &str, value: &str) -> Result<()> {
    ensure!(!field.is_empty() && field.len() <= 128, "invalid field");
    let fields = map(doc, &object(doc, id)?, "fields")?;
    let mut tx = doc.transaction();
    tx.put(fields, field, value)?;
    tx.commit();
    Ok(())
}

pub fn values(doc: &Automerge, id: &str, field: &str) -> Result<Vec<String>> {
    Ok(doc
        .get_all(map(doc, &object(doc, id)?, "fields")?, field)?
        .into_iter()
        .filter_map(|(v, _)| v.as_str().map(str::to_owned))
        .collect())
}

pub fn resolve(doc: &mut Automerge, id: &str, field: &str, chosen: &str) -> Result<()> {
    let obj = object(doc, id)?;
    let fields = map(doc, &obj, "fields")?;
    let recovery = map(doc, &obj, "recovery")?;
    let record = serde_json::to_string(
        &serde_json::json!({"field":field,"values":values(doc,id,field)?,"chosen":chosen}),
    )
    .map_err(|_| Failure(Reason::Protocol))?;
    let mut tx = doc.transaction();
    tx.put(recovery, token(), record)?;
    tx.put(fields, field, chosen)?;
    tx.commit();
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Recovery {
    pub field: String,
    pub values: Vec<String>,
    pub chosen: String,
}

pub fn recovery(doc: &Automerge, id: &str) -> Result<Vec<Recovery>> {
    let recovery = map(doc, &object(doc, id)?, "recovery")?;
    doc.keys(&recovery)
        .map(|key| {
            let value = doc
                .get(&recovery, key)?
                .context("missing recovery entry")?
                .0;
            Ok(
                serde_json::from_str(value.as_str().context("invalid recovery entry")?)
                    .map_err(|_| Failure(Reason::CorruptDocument))?,
            )
        })
        .collect()
}

fn token() -> String {
    let mut bytes = [0; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn check(doc: &mut Automerge, id: &str) -> Result<()> {
    let checks = map(doc, &object(doc, id)?, "checked_tokens")?;
    let mut tx = doc.transaction();
    tx.put(checks, token(), true)?;
    tx.commit();
    Ok(())
}

pub fn uncheck(doc: &mut Automerge, id: &str) -> Result<()> {
    let checks = map(doc, &object(doc, id)?, "checked_tokens")?;
    let observed: Vec<_> = doc.keys(&checks).collect();
    let mut tx = doc.transaction();
    for key in observed {
        tx.delete(&checks, key)?;
    }
    tx.commit();
    Ok(())
}

pub fn checked(doc: &Automerge, id: &str) -> Result<bool> {
    Ok(doc
        .keys(map(doc, &object(doc, id)?, "checked_tokens")?)
        .next()
        .is_some())
}

pub fn splice(doc: &mut Automerge, id: &str, at: usize, delete: isize, text: &str) -> Result<()> {
    let title = doc
        .get(object(doc, id)?, "title")?
        .context("missing title")?
        .1;
    let mut tx = doc.transaction();
    tx.splice_text(title, at, delete, text)?;
    tx.commit();
    Ok(())
}

pub fn text(doc: &Automerge, id: &str) -> Result<String> {
    Ok(doc.text(
        doc.get(object(doc, id)?, "title")?
            .context("missing title")?
            .1,
    )?)
}

/// Tombstones hide objects without unlinking their history, including concurrent edits.
/// Every deleted object's latest merged fields/text remain available through these helpers.
pub fn delete(doc: &mut Automerge, id: &str) -> Result<()> {
    let deleted = map(doc, &object(doc, id)?, "deleted_tokens")?;
    let mut tx = doc.transaction();
    tx.put(deleted, token(), true)?;
    tx.commit();
    Ok(())
}

pub fn deleted(doc: &Automerge, id: &str) -> Result<bool> {
    Ok(doc
        .keys(map(doc, &object(doc, id)?, "deleted_tokens")?)
        .next()
        .is_some())
}

pub fn restore(doc: &mut Automerge, id: &str) -> Result<()> {
    let deleted = map(doc, &object(doc, id)?, "deleted_tokens")?;
    let observed: Vec<_> = doc.keys(&deleted).collect();
    let mut tx = doc.transaction();
    for key in observed {
        tx.delete(&deleted, key)?;
    }
    tx.commit();
    Ok(())
}
