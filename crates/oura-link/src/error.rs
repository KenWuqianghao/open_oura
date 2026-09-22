//! Error and result types for the BLE link layer.
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("ble error: {0}")]
    Ble(String),
    #[error("no matching Oura ring found")]
    DeviceNotFound,
    #[error("timed out connecting to the ring (is it still linked to a phone? take it off/on the charger and retry)")]
    ConnectTimeout,
    #[error("characteristic not found: {0}")]
    CharacteristicNotFound(String),
    /// The ring gave a verdict and it was not `Success`.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// The ring gave no verdict at all (no nonce, or no reply to the challenge).
    /// This is a link problem, not a key problem: a Ring 3 drops the link about
    /// two seconds after a configuration write, so the caller should reconnect.
    #[error("no auth reply from the ring: {0}")]
    NoAuthReply(String),
    /// Pairing was attempted on a ring that already holds a key. Only a
    /// factory-reset ring accepts `SetAuthKey`.
    #[error("ring is not in factory-reset state (auth state {state:#04x}: {reason}); factory-reset it first")]
    NotFactoryReset { state: u8, reason: &'static str },
    /// The ring answered `SetAuthKey` with a non-zero status byte. `0x05` is
    /// "production tests missing" in the official app.
    #[error("set_auth_key rejected with status {0:#04x}")]
    SetKeyRejected(u8),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(feature = "ble")]
impl From<btleplug::Error> for Error {
    fn from(e: btleplug::Error) -> Self {
        Error::Ble(e.to_string())
    }
}
