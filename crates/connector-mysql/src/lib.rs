mod metadata;

use envlume_application::{
    ErrorCode,
    credentials::Catalogue,
    environments::RemoteAction,
    input::TableName,
    mysql_policy,
    ports::AuthorizedResource,
    remote::{DatabaseBackend, PoolIdentity, RemoteFault as Fault},
    result::{ConnectionResult, Health, Table},
};
use envlume_domain::{ResourceBinding, ResourceId, ResourceKind, TlsMode};
use envlume_secrets::NativeVault;
use mysql_async::{
    Conn, DriverError, Error as Driver, IoError, OptsBuilder, Pool, PoolConstraints, PoolOpts,
    SslOpts, prelude::Queryable,
};
use std::{
    collections::{BTreeSet, HashMap},
    net::IpAddr,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    runtime::Runtime,
    sync::Semaphore,
    time::{Instant as Deadline, timeout, timeout_at},
};

const SLOTS: usize = 8;
const CONNECTIONS: usize = 2;
const HEALTH_RECORDS: usize = 64;
const IDLE: Duration = Duration::from_secs(30);
const BUDGET: Duration = Duration::from_secs(5);
const RESPONSE_BYTES: usize = 256 * 1024;

/// Adapter boundary for the native vault; only an authorized request can submit a login.
pub trait Logins: Send + Sync {
    fn with_login<T>(
        &self,
        r: &AuthorizedResource<'_>,
        use_login: impl FnOnce(&str, &str) -> Result<T, ErrorCode>,
    ) -> Result<T, ErrorCode>;
}
impl<C: Catalogue + Send + Sync> Logins for NativeVault<C> {
    fn with_login<T>(
        &self,
        r: &AuthorizedResource<'_>,
        f: impl FnOnce(&str, &str) -> Result<T, ErrorCode>,
    ) -> Result<T, ErrorCode> {
        NativeVault::with_login(self, r, f)
    }
}
fn runtime() -> Result<&'static Runtime, Fault> {
    static RT: OnceLock<Result<Runtime, ()>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| ())
    })
    .as_ref()
    .map_err(|_| Fault::Service)
}
struct Slot {
    resource: ResourceBinding,
    ips: Vec<IpAddr>,
    pool: Option<Pool>,
    healthy: Option<bool>,
    touched: Instant,
}
type Slots = HashMap<PoolIdentity, Slot>;
fn retire(pool: Pool) {
    if let Ok(rt) = runtime() {
        rt.spawn(async move {
            let _ = pool.disconnect().await;
        });
    }
}
fn remove(slots: &mut Slots, keep: impl Fn(PoolIdentity, &Slot) -> bool) {
    slots.retain(|k, s| {
        let retain = keep(*k, s);
        if !retain && let Some(pool) = s.pool.take() {
            retire(pool);
        }
        retain
    })
}

pub struct MySqlConnector<L> {
    login: L,
    slots: Arc<Mutex<Slots>>,
    inflight: Semaphore,
}
impl<L: Logins> MySqlConnector<L> {
    pub fn new(login: L) -> Result<Self, Fault> {
        let slots = Arc::new(Mutex::new(Slots::new()));
        let weak = Arc::downgrade(&slots);
        runtime()?.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(map) = weak.upgrade() else { break };
                if let Ok(mut slots) = map.lock() {
                    remove(&mut slots, |_, s| s.touched.elapsed() < IDLE);
                }
            }
        });
        Ok(Self {
            login,
            slots,
            inflight: Semaphore::new(SLOTS),
        })
    }
    fn pool(
        &self,
        key: PoolIdentity,
        r: &ResourceBinding,
        ips: Vec<IpAddr>,
        options: OptsBuilder,
    ) -> Result<Pool, Fault> {
        let mut slots = self.slots.lock().map_err(|_| Fault::Service)?;
        if self.inflight.is_closed() {
            return Err(Fault::Service);
        }
        if slots.get(&key).is_none_or(|s| s.pool.is_none())
            && slots.values().filter(|s| s.pool.is_some()).count() >= SLOTS
        {
            return Err(Fault::Busy);
        }
        let entry = slots.entry(key).or_insert_with(|| Slot {
            resource: r.clone(),
            ips: vec![],
            pool: None,
            healthy: None,
            touched: Instant::now(),
        });
        if entry.resource != *r || entry.ips != ips {
            if let Some(pool) = entry.pool.take() {
                retire(pool);
            }
            entry.resource = r.clone();
            entry.ips = ips;
        }
        entry.touched = Instant::now();
        Ok(entry.pool.get_or_insert_with(|| Pool::new(options)).clone())
    }
    fn mark(&self, key: PoolIdentity, r: &ResourceBinding, ok: bool) {
        if let Ok(mut slots) = self.slots.lock() {
            if self.inflight.is_closed() {
                return;
            }
            if slots.len() >= HEALTH_RECORDS
                && !slots.contains_key(&key)
                && let Some(old) = slots
                    .iter()
                    .filter(|(_, s)| s.pool.is_none())
                    .min_by_key(|(_, s)| s.touched)
                    .map(|(k, _)| *k)
            {
                slots.remove(&old);
            }
            let entry = slots.entry(key).or_insert_with(|| Slot {
                resource: r.clone(),
                ips: vec![],
                pool: None,
                healthy: None,
                touched: Instant::now(),
            });
            if entry.resource == *r {
                entry.touched = Instant::now();
                entry.healthy = Some(ok);
                if !ok && let Some(pool) = entry.pool.take() {
                    retire(pool);
                }
            }
        }
    }
    fn perform<T>(
        &self,
        auth: &AuthorizedResource<'_>,
        action: RemoteAction,
        op: impl AsyncFnOnce(&mut Conn, &ResourceBinding) -> Result<T, Fault>,
    ) -> Result<T, Fault> {
        let _permit = self.inflight.try_acquire().map_err(|_| Fault::Busy)?;
        let key = auth.pool_identity(action).map_err(Fault::Access)?;
        let r = auth.binding().map_err(Fault::Access)?;
        let route = r.route.as_ref().ok_or(Fault::Address)?;
        mysql_policy::validate(r, route, &r.tables).map_err(|_| Fault::Address)?;
        let database = r.database.as_ref().ok_or(Fault::Address)?;
        let rt = runtime()?;
        let end = Deadline::now() + BUDGET;
        let result = (|| {
            let ips = rt.block_on(async {
                let resolved = timeout_at(end, tokio::net::lookup_host((r.host.as_str(), r.port)))
                    .await
                    .map_err(|_| Fault::Timeout)?
                    .map_err(|_| Fault::Dns)?;
                let ips: BTreeSet<_> = resolved.map(|a| a.ip()).collect();
                if ips.is_empty() || !ips.is_subset(&route.ips) {
                    return Err(Fault::Address);
                }
                Ok(ips.into_iter().collect::<Vec<_>>())
            })?;
            let options = self
                .login
                .with_login(auth, |u, p| {
                    let ssl = match r.tls {
                        TlsMode::Disabled => None,
                        TlsMode::VerifyIdentity => {
                            let mut opts = SslOpts::default();
                            if let Some(ca) = &route.ca_pem {
                                opts = opts
                                    .with_disable_built_in_roots(true)
                                    .with_root_certs(vec![ca.as_bytes().to_vec().into()]);
                            }
                            Some(opts)
                        }
                    };
                    let pool = PoolOpts::default()
                        .with_constraints(
                            PoolConstraints::new(0, CONNECTIONS).expect("valid limits"),
                        )
                        .with_inactive_connection_ttl(IDLE)
                        .with_ttl_check_interval(Duration::from_secs(1));
                    Ok(OptsBuilder::default()
                        .ip_or_hostname(r.host.clone())
                        .tcp_port(r.port)
                        .resolved_ips(Some(ips.clone()))
                        .prefer_socket(false)
                        .user(Some(u))
                        .pass(Some(p))
                        .db_name(Some(database.as_str()))
                        .ssl_opts(ssl)
                        .pool_opts(pool)
                        .max_allowed_packet(Some(RESPONSE_BYTES))
                        .wait_timeout(Some(30)))
                })
                .map_err(Fault::Access)?;
            rt.block_on(async {
                if Deadline::now() >= end {
                    return Err(Fault::Timeout);
                }
                auth.binding().map_err(Fault::Access)?;
                let pool = self.pool(key, r, ips, options)?;
                timeout_at(end, async {
                    let mut conn = pool.get_conn().await.map_err(classify)?;
                    auth.binding().map_err(Fault::Access)?;
                    let value = op(&mut conn, r).await?;
                    auth.binding().map_err(Fault::Access)?;
                    Ok(value)
                })
                .await
                .map_err(|_| Fault::Timeout)?
            })
        })();
        self.mark(key, r, result.is_ok());
        result
    }
}
async fn rows<T: mysql_async::prelude::FromRow>(
    conn: &mut Conn,
    stmt: &str,
    db: &str,
    table: &str,
    names: &std::collections::BTreeSet<String>,
    remaining: &mut usize,
) -> Result<Vec<T>, Fault> {
    if names.is_empty() {
        return Ok(vec![]);
    }
    let names = names
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",");
    let mut stream = conn
        .exec_iter(stmt, (db, table, names))
        .await
        .map_err(classify)?;
    let mut out = Vec::new();
    while let Some(row) = stream.next().await.map_err(classify)? {
        if out.len() >= 1024 {
            return Err(Fault::Limit);
        }
        for i in 0..row.len() {
            let len = match row.as_ref(i) {
                Some(mysql_async::Value::Bytes(b)) => b.len() + 8,
                _ => 16,
            };
            *remaining = remaining.checked_sub(len).ok_or(Fault::Limit)?;
        }
        out.push(mysql_async::from_row_opt(row).map_err(|_| Fault::Service)?);
    }
    Ok(out)
}
fn classify(error: Driver) -> Fault {
    match error {
        Driver::Io(IoError::Tls(_)) | Driver::Driver(DriverError::NoClientSslFlagFromServer) => {
            Fault::Tls
        }
        Driver::Io(IoError::Io(e)) if e.kind() == std::io::ErrorKind::InvalidData => Fault::Service,
        Driver::Io(IoError::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut => Fault::Timeout,
        Driver::Io(_) => Fault::Tcp,
        Driver::Server(e) if e.code == 1045 => Fault::Authentication,
        Driver::Server(e) if matches!(e.code, 1044 | 1142 | 1143) => Fault::Privileges,
        Driver::Driver(DriverError::CantParseServerVersion { .. }) => Fault::Version,
        Driver::Driver(DriverError::PacketTooLarge) => Fault::Limit,
        _ => Fault::Service,
    }
}
impl<L: Logins> DatabaseBackend for MySqlConnector<L> {
    fn kind(&self) -> ResourceKind {
        ResourceKind::MySql
    }
    fn test(&self, r: &AuthorizedResource<'_>) -> Result<ConnectionResult, Fault> {
        let start = Instant::now();
        self.perform(r, RemoteAction::Connect, async |conn, _| {
            conn.ping().await.map_err(classify)?;
            let (a, b, c) = conn.server_version();
            Ok(ConnectionResult {
                latency_ms: start.elapsed().as_millis() as u64,
                version_summary: format!("MySQL {a}.{b}.{c}"),
            })
        })
    }
    fn schema(
        &self,
        r: &AuthorizedResource<'_>,
        only: Option<&TableName>,
    ) -> Result<Vec<Table>, Fault> {
        let scope = r.binding().map_err(Fault::Access)?;
        if scope.tables.is_empty() || only.is_some_and(|t| !scope.tables.contains_key(t.as_str())) {
            return Err(Fault::Access(ErrorCode::TargetNotAllowed));
        }
        self.perform(r, RemoteAction::Schema, async |conn, scope| {
            let db = scope.database.as_deref().ok_or(Fault::Address)?;
            let mut tables = Vec::new();
            let mut remaining = RESPONSE_BYTES;
            for (name, allowed) in &scope.tables {
                if only.is_some_and(|t| t.as_str() != name) {
                    continue;
                }
                r.binding().map_err(Fault::Access)?;
                let cols = rows(
                    conn,
                    metadata::COLUMNS,
                    db,
                    name,
                    &allowed.columns,
                    &mut remaining,
                )
                .await?;
                r.binding().map_err(Fault::Access)?;
                let indexes = rows(
                    conn,
                    metadata::INDEXES,
                    db,
                    name,
                    &allowed.indexes,
                    &mut remaining,
                )
                .await?;
                if let Some(table) = metadata::project(name, allowed, cols, indexes)? {
                    tables.push(table);
                }
            }
            // MCP returns structuredContent and its escaped text representation.
            let encoded = serde_json::to_string(&tables).map_err(|_| Fault::Service)?;
            let text = serde_json::to_vec(&encoded).map_err(|_| Fault::Service)?;
            if encoded.len() + text.len() > RESPONSE_BYTES - 4096 {
                return Err(Fault::Limit);
            }
            Ok(tables)
        })
    }
    fn retain(&self, live: &dyn Fn(PoolIdentity, &ResourceBinding) -> bool) {
        if let Ok(mut slots) = self.slots.lock() {
            remove(&mut slots, |k, s| live(k, &s.resource));
        }
    }
    fn health(&self, binding: envlume_domain::BindingId, id: ResourceId) -> Health {
        self.slots
            .lock()
            .ok()
            .and_then(|s| {
                s.iter()
                    .filter(|(k, _)| k.binding == binding && k.resource == id)
                    .max_by_key(|(_, s)| s.touched)
                    .and_then(|(_, s)| s.healthy)
            })
            .map_or(Health::Unknown, |ok| {
                if ok {
                    Health::Healthy
                } else {
                    Health::Unavailable
                }
            })
    }
    fn close(&self) -> Result<(), Fault> {
        self.inflight.close();
        let pools: Vec<_> = self
            .slots
            .lock()
            .map_err(|_| Fault::Service)?
            .drain()
            .filter_map(|(_, s)| s.pool)
            .collect();
        runtime()?.block_on(async {
            let mut closing = tokio::task::JoinSet::new();
            for p in pools {
                closing.spawn(p.disconnect());
            }
            timeout(BUDGET, async {
                while let Some(r) = closing.join_next().await {
                    r.map_err(|_| Fault::Service)?.map_err(classify)?;
                }
                Ok(())
            })
            .await
            .map_err(|_| Fault::Timeout)?
        })
    }
}
impl<L> Drop for MySqlConnector<L> {
    fn drop(&mut self) {
        if let Ok(mut s) = self.slots.lock() {
            remove(&mut s, |_, _| false);
        }
    }
}
