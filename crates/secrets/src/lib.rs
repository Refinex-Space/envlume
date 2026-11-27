//! Native credential adapters. Only a currently authorized connector callback sees a login.
use envlume_application::{
    ErrorCode,
    credentials::{self, Catalogue, Change, CredentialWriter, Fault, ItemKey, Saved, Selection},
    ports::{AuthorizedResource, SecretStore},
};
use sha2::{Digest, Sha256};
use std::path::Path;
use zeroize::Zeroizing;

mod platform;
#[cfg(test)]
mod tests;

// The common limit is Windows CRED_MAX_CREDENTIAL_BLOB_SIZE. The header carries
// a format version and the byte length of the UTF-8 username.
const MAX_BLOB: usize = 2560;
const HEADER: &[u8; 4] = b"ELC1";
pub struct Login(Zeroizing<Vec<u8>>);
impl Login {
    pub fn new(user: &str, password: &str) -> Result<Self, Fault> {
        if user.is_empty()
            || user.contains('\0')
            || password.contains('\0')
            || user.len().saturating_add(password.len()) > MAX_BLOB - 6
        {
            return Err(Fault::InvalidInput);
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(6 + user.len() + password.len()));
        bytes.extend_from_slice(HEADER);
        bytes.extend_from_slice(&(user.len() as u16).to_be_bytes());
        bytes.extend_from_slice(user.as_bytes());
        bytes.extend_from_slice(password.as_bytes());
        Ok(Self(bytes))
    }
    fn fields(&self) -> Result<(&str, &str), Fault> {
        let raw = &self.0;
        if raw.len() < 7 || raw.len() > MAX_BLOB || !raw.starts_with(HEADER) {
            return Err(Fault::Damaged);
        }
        let end = 6 + usize::from(u16::from_be_bytes([raw[4], raw[5]]));
        if end == 6 || end > raw.len() || raw[6..].contains(&0) {
            return Err(Fault::Damaged);
        }
        let decode = |bytes| std::str::from_utf8(bytes).map_err(|_| Fault::Damaged);
        Ok((decode(&raw[6..end])?, decode(&raw[end..])?))
    }
}

pub struct NativeVault {
    namespace: String,
}
impl NativeVault {
    pub fn for_catalogue(root: &Path) -> Result<Self, Fault> {
        let path = root.canonicalize().map_err(|_| Fault::Storage)?;
        let digest = Sha256::digest(path.as_os_str().as_encoded_bytes());
        Ok(Self {
            namespace: format!("io.envlume.credentials.v1.{digest:x}"),
        })
    }
    pub fn status(&self, db: &impl Catalogue, selection: &Selection) -> Result<(), Fault> {
        credentials::status(db, &Writer(self), selection)
    }
    pub fn change(
        &self,
        db: &impl Catalogue,
        selection: &Selection,
        action: Change,
        input: Option<&Login>,
    ) -> Result<Saved, Fault> {
        credentials::change(db, &Writer(self), selection, action, input)
    }
    pub fn cleanup(
        &self,
        db: &impl Catalogue,
        binding: envlume_domain::BindingId,
        origin: &Path,
    ) -> Result<(), Fault> {
        credentials::cleanup(db, &Writer(self), binding, origin)
    }
    pub fn with_login<T>(
        &self,
        resource: &AuthorizedResource<'_>,
        use_login: impl FnOnce(&str, &str) -> Result<T, ErrorCode>,
    ) -> Result<T, ErrorCode> {
        let key = ItemKey::for_resource(resource.binding()?)
            .map_err(|_| ErrorCode::CredentialUnavailable)?;
        let login = platform::read(&self.namespace, &key.reference())
            .map_err(|_| ErrorCode::CredentialUnavailable)?;
        let (user, password) = login
            .fields()
            .map_err(|_| ErrorCode::CredentialUnavailable)?;
        // OS access may have waited for consent while the binding/grant changed.
        resource.binding()?;
        use_login(user, password)
    }
}
struct Writer<'a>(&'a NativeVault);
impl CredentialWriter for Writer<'_> {
    type Input = Login;
    fn write_new(&self, key: &ItemKey, input: &Login) -> Result<(), Fault> {
        platform::write(&self.0.namespace, &key.reference(), &input.0)
    }
    fn available(&self, key: &ItemKey) -> Result<(), Fault> {
        platform::read(&self.0.namespace, &key.reference())?
            .fields()
            .map(|_| ())
    }
    fn delete(&self, key: &ItemKey) -> Result<(), Fault> {
        platform::delete(&self.0.namespace, &key.reference())
    }
}
impl SecretStore for NativeVault {
    fn check_reference(&self, resource: &AuthorizedResource<'_>) -> Result<(), ErrorCode> {
        self.with_login(resource, |_, _| Ok(()))
    }
}
