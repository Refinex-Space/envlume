use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! ids {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn generate() -> Self { Self(Uuid::new_v4()) }
        }
    )+};
}

ids!(
    ProjectId,
    BindingId,
    ProfileId,
    TargetId,
    ResourceId,
    CredentialId,
    TaskId,
    GrantId,
    ExecutionId,
    IntegrationId,
    RequestId,
    EvidenceId
);

/// Local logical name. The same spelling may occur under different parents.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(try_from = "String")]
pub struct Alias(String);

impl TryFrom<String> for Alias {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let bytes = value.as_bytes();
        if !(1..=64).contains(&bytes.len())
            || !bytes[0].is_ascii_lowercase()
            || !bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
        {
            return Err(
                "expected a logical alias of 1–64 lowercase ASCII letters, digits, underscores or hyphens, starting with a letter",
            );
        }
        Ok(Self(value))
    }
}

impl Alias {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
