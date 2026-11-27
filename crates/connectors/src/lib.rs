//! The installed registry exposes only statically typed operations.
use envlume_application::{
    input::TableName,
    ports::AuthorizedResource,
    remote::{DatabaseBackend, PoolIdentity, RemoteFault},
    result::{ConnectionResult, Health, Table},
};
use envlume_domain::{ResourceBinding, ResourceId, ResourceKind};

pub struct Registry {
    mysql: Box<dyn DatabaseBackend>,
}
impl Registry {
    pub fn new(connector: impl DatabaseBackend + 'static) -> Self {
        match connector.kind() {
            ResourceKind::MySql => Self {
                mysql: Box::new(connector),
            },
        }
    }
    fn select(&self, r: &AuthorizedResource<'_>) -> Result<&dyn DatabaseBackend, RemoteFault> {
        match r.binding().map_err(RemoteFault::Access)?.kind {
            ResourceKind::MySql => Ok(self.mysql.as_ref()),
        }
    }
}
impl DatabaseBackend for Registry {
    fn kind(&self) -> ResourceKind {
        ResourceKind::MySql
    }
    fn capabilities(&self) -> &'static [envlume_application::environments::RemoteAction] {
        self.mysql.capabilities()
    }
    fn query(
        &self,
        q: &envlume_application::remote::AuthorizedQuery<'_>,
    ) -> Result<envlume_application::result::QueryResult, RemoteFault> {
        self.select(q.resource())?.query(q)
    }
    fn test(&self, r: &AuthorizedResource<'_>) -> Result<ConnectionResult, RemoteFault> {
        self.select(r)?.test(r)
    }
    fn schema(
        &self,
        r: &AuthorizedResource<'_>,
        table: Option<&TableName>,
    ) -> Result<Vec<Table>, RemoteFault> {
        self.select(r)?.schema(r, table)
    }
    fn retain(&self, live: &dyn Fn(PoolIdentity, &ResourceBinding) -> bool) {
        self.mysql.retain(live)
    }
    fn health(&self, binding: envlume_domain::BindingId, r: ResourceId) -> Health {
        self.mysql.health(binding, r)
    }
    fn close(&self) -> Result<(), RemoteFault> {
        self.mysql.close()
    }
}
