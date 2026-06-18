use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct CredentialResolver {
    secrets: BTreeMap<String, String>,
}

impl CredentialResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_secret(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into().trim().to_string();
        if !name.is_empty() {
            self.secrets.insert(name, value.into());
        }
        self
    }

    pub fn resolve(&self, credential_ref: &str) -> Result<String, CredentialResolutionError> {
        let credential_ref = credential_ref.trim();
        if let Some(env_name) = credential_ref.strip_prefix("env:") {
            let env_name = env_name.trim();
            if env_name.is_empty() {
                return Err(CredentialResolutionError::InvalidReference(
                    "env credential reference requires a variable name".to_string(),
                ));
            }
            return std::env::var(env_name).map_err(|_| {
                CredentialResolutionError::MissingCredential(format!(
                    "missing environment credential {env_name}"
                ))
            });
        }

        for prefix in ["secret:", "secret-store:"] {
            if let Some(secret_name) = credential_ref.strip_prefix(prefix) {
                let secret_name = secret_name.trim();
                if secret_name.is_empty() {
                    return Err(CredentialResolutionError::InvalidReference(format!(
                        "{prefix} credential reference requires a secret name"
                    )));
                }
                return self.secrets.get(secret_name).cloned().ok_or_else(|| {
                    CredentialResolutionError::MissingCredential(format!(
                        "missing secret-store credential {secret_name}"
                    ))
                });
            }
        }

        Err(CredentialResolutionError::UnsupportedReference(format!(
            "unsupported credential reference {credential_ref}; expected env:NAME, secret:NAME, or secret-store:NAME"
        )))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CredentialResolutionError {
    InvalidReference(String),
    MissingCredential(String),
    UnsupportedReference(String),
}

impl CredentialResolutionError {
    pub fn detail(&self) -> String {
        match self {
            Self::InvalidReference(detail)
            | Self::MissingCredential(detail)
            | Self::UnsupportedReference(detail) => detail.clone(),
        }
    }
}
