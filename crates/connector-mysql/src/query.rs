use super::{Fault, classify};
use envlume_application::{
    remote::{Parameter, ReadQuery},
    result::{Cell, QueryResult},
};
use envlume_domain::QueryPolicy;
use mysql_async::{Conn, Params, TxOpts, Value, prelude::Queryable};

async fn require_reader(conn: &mut Conn) -> Result<(), Fault> {
    let role: Option<String> = conn
        .query_first("SELECT CURRENT_ROLE()")
        .await
        .map_err(classify)?;
    if role.as_deref() != Some("NONE") {
        return Err(Fault::Privileges);
    }
    let mut grants = conn
        .query_iter("SHOW GRANTS FOR CURRENT_USER")
        .await
        .map_err(classify)?;
    let mut select = false;
    while let Some(row) = grants.next().await.map_err(classify)? {
        let grant: String = mysql_async::from_row_opt(row).map_err(|_| Fault::Privileges)?;
        if grant.contains(" WITH GRANT OPTION") {
            return Err(Fault::Privileges);
        }
        if grant.starts_with("GRANT SELECT ON ") {
            select = true;
        } else if !grant.starts_with("GRANT USAGE ON ") {
            return Err(Fault::Privileges);
        }
    }
    if !select {
        return Err(Fault::Privileges);
    }
    Ok(())
}
fn cell(v: Value) -> Result<Cell, Fault> {
    Ok(match v {
        Value::NULL => Cell::Null,
        Value::Int(n) => Cell::Number(n.into()),
        Value::UInt(n) => Cell::Number(n.into()),
        Value::Float(n) => {
            Cell::Number(serde_json::Number::from_f64(n.into()).ok_or(Fault::Service)?)
        }
        Value::Double(n) => Cell::Number(serde_json::Number::from_f64(n).ok_or(Fault::Service)?),
        Value::Bytes(b) => Cell::Text(String::from_utf8(b).map_err(|_| Fault::Limit)?),
        Value::Date(y, m, d, h, min, s, us) => Cell::Text(format!(
            "{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}.{us:06}"
        )),
        Value::Time(neg, days, h, m, s, us) => Cell::Text(format!(
            "{}{:02}:{m:02}:{s:02}.{us:06}",
            if neg { "-" } else { "" },
            u64::from(days) * 24 + u64::from(h)
        )),
    })
}
// rmcp emits both structuredContent and an escaped JSON text block. The remaining
// 2048 bytes cover both fixed report envelopes, two 64-byte aliases and a bounded RPC id.
fn wire_cost<T: serde::Serialize + ?Sized>(value: &T) -> Result<usize, Fault> {
    let raw = serde_json::to_string(value).map_err(|_| Fault::Service)?;
    Ok(raw.len() + serde_json::to_vec(&raw).map_err(|_| Fault::Service)?.len())
}

pub(super) async fn read(
    conn: &mut Conn,
    plan: &ReadQuery,
    policy: &QueryPolicy,
    authorize: impl Fn() -> Result<(), Fault>,
) -> Result<QueryResult, Fault> {
    let mut remaining = policy
        .max_bytes
        .checked_sub(2048 + wire_cost(plan.columns())?)
        .ok_or(Fault::Limit)?;
    require_reader(conn).await?;
    // MAX_EXECUTION_TIME covers the SELECT. Metadata lock acquisition during PREPARE
    // has its own server timeout, including when the client deadline expires first.
    conn.query_drop(format!(
        "SET SESSION lock_wait_timeout={}",
        policy.timeout_ms.div_ceil(1000)
    ))
    .await
    .map_err(classify)?;
    let mut options = TxOpts::default();
    options.with_readonly(true);
    let mut tx = conn.start_transaction(options).await.map_err(classify)?;
    let args = plan
        .parameters()
        .iter()
        .map(|p| match p {
            Parameter::Integer(n) => Value::Int(*n),
            Parameter::Text(s) => Value::from(s.as_str()),
        })
        .collect();
    authorize()?;
    let mut stream = tx
        .exec_iter(plan.statement(), Params::Positional(args))
        .await
        .map_err(classify)?;
    let mut rows = Vec::new();
    let mut truncated = false;
    while let Some(row) = stream.next().await.map_err(classify)? {
        let mut values = row.unwrap();
        let marker = values.pop().ok_or(Fault::Service)?;
        let oversize = mysql_async::from_value_opt::<bool>(marker).map_err(|_| Fault::Service)?;
        if oversize || rows.len() == policy.max_rows {
            truncated = true;
            break;
        }
        let row = values
            .into_iter()
            .map(cell)
            .collect::<Result<Vec<_>, _>>()?;
        let cost = wire_cost(&row)? + 2;
        if cost > remaining {
            truncated = true;
            break;
        }
        remaining -= cost;
        rows.push(row);
    }
    drop(stream);
    if !truncated {
        tx.rollback().await.map_err(classify)?;
    }
    let mut result =
        QueryResult::new(plan.columns().to_vec(), rows, truncated).map_err(Fault::Access)?;
    result.record_masking(plan.masked());
    Ok(result)
}
