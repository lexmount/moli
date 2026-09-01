use std::fmt;

use moli_storage_key::MoliStorageKey;

/// Classic versus module SharedWorker script execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SharedWorkerScriptType {
    Classic,
    Module,
}

/// Request credentials mode used for module worker script loading.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SharedWorkerCredentialsMode {
    Omit,
    SameOrigin,
    Include,
}

/// Secure-context class of the environment that called the constructor.
///
/// Chromium rejects reuse when a secure creator and non-secure creator try to
/// connect to the same SharedWorker key. This remains compatibility state, not
/// part of the matching key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SharedWorkerCreationContextType {
    Secure,
    Nonsecure,
}

/// Chromium includes same-site-cookie behavior in the matching key.
///
/// The default is derived from the constructor `StorageKey`: first-party
/// contexts get `All`, while third-party contexts get `None`. Third-party
/// contexts cannot explicitly request `All`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SharedWorkerSameSiteCookies {
    All,
    None,
}

impl SharedWorkerSameSiteCookies {
    /// Return the Chromium-style default for a constructor storage key.
    pub fn default_for_storage_key(storage_key: &MoliStorageKey) -> Self {
        if storage_key.is_third_party_partitioned() {
            Self::None
        } else {
            Self::All
        }
    }

    /// Return whether the requested mode may be used for this storage key.
    pub fn is_allowed_for_storage_key(self, storage_key: &MoliStorageKey) -> bool {
        !matches!(self, Self::All) || !storage_key.is_third_party_partitioned()
    }
}

/// Creation options that must remain compatible for an existing worker slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedWorkerDescriptor {
    script_type: SharedWorkerScriptType,
    credentials_mode: SharedWorkerCredentialsMode,
    extended_lifetime: bool,
    creation_context_type: SharedWorkerCreationContextType,
}

impl SharedWorkerDescriptor {
    /// Build a descriptor from parsed constructor options and the constructor
    /// environment.
    pub fn new(
        script_type: SharedWorkerScriptType,
        credentials_mode: SharedWorkerCredentialsMode,
        extended_lifetime: bool,
        creation_context_type: SharedWorkerCreationContextType,
    ) -> Self {
        Self {
            script_type,
            credentials_mode,
            extended_lifetime,
            creation_context_type,
        }
    }

    /// Return whether the worker is classic or module.
    pub fn script_type(&self) -> SharedWorkerScriptType {
        self.script_type
    }

    /// Return the requested credentials mode.
    pub fn credentials_mode(&self) -> SharedWorkerCredentialsMode {
        self.credentials_mode
    }

    /// Return whether the constructor requested an extended worker lifetime.
    pub fn extended_lifetime(&self) -> bool {
        self.extended_lifetime
    }

    /// Return the secure-context class of the constructor environment.
    pub fn creation_context_type(&self) -> SharedWorkerCreationContextType {
        self.creation_context_type
    }

    /// Check compatibility with an already-created SharedWorker.
    pub fn ensure_compatible_with(
        &self,
        requested: &Self,
    ) -> Result<(), SharedWorkerCompatibilityError> {
        if self.script_type != requested.script_type {
            return Err(SharedWorkerCompatibilityError::ScriptType {
                existing: self.script_type,
                requested: requested.script_type,
            });
        }
        if self.credentials_mode != requested.credentials_mode {
            return Err(SharedWorkerCompatibilityError::CredentialsMode {
                existing: self.credentials_mode,
                requested: requested.credentials_mode,
            });
        }
        if self.extended_lifetime != requested.extended_lifetime {
            return Err(SharedWorkerCompatibilityError::ExtendedLifetime {
                existing: self.extended_lifetime,
                requested: requested.extended_lifetime,
            });
        }
        if self.creation_context_type != requested.creation_context_type {
            return Err(SharedWorkerCompatibilityError::CreationContextType {
                existing: self.creation_context_type,
                requested: requested.creation_context_type,
            });
        }
        Ok(())
    }
}

impl Default for SharedWorkerDescriptor {
    fn default() -> Self {
        Self::new(
            SharedWorkerScriptType::Classic,
            SharedWorkerCredentialsMode::SameOrigin,
            false,
            SharedWorkerCreationContextType::Secure,
        )
    }
}

/// Incompatible constructor options for an existing SharedWorker key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharedWorkerCompatibilityError {
    ScriptType {
        existing: SharedWorkerScriptType,
        requested: SharedWorkerScriptType,
    },
    CredentialsMode {
        existing: SharedWorkerCredentialsMode,
        requested: SharedWorkerCredentialsMode,
    },
    ExtendedLifetime {
        existing: bool,
        requested: bool,
    },
    CreationContextType {
        existing: SharedWorkerCreationContextType,
        requested: SharedWorkerCreationContextType,
    },
}

impl fmt::Display for SharedWorkerCompatibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScriptType {
                existing,
                requested,
            } => write!(
                f,
                "existing SharedWorker script type {existing:?} is incompatible with requested {requested:?}"
            ),
            Self::CredentialsMode {
                existing,
                requested,
            } => write!(
                f,
                "existing SharedWorker credentials mode {existing:?} is incompatible with requested {requested:?}"
            ),
            Self::ExtendedLifetime {
                existing,
                requested,
            } => write!(
                f,
                "existing SharedWorker extended lifetime {existing} is incompatible with requested {requested}"
            ),
            Self::CreationContextType {
                existing,
                requested,
            } => write!(
                f,
                "existing SharedWorker creation context {existing:?} is incompatible with requested {requested:?}"
            ),
        }
    }
}

impl std::error::Error for SharedWorkerCompatibilityError {}
