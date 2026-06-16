use crate::types::Credential;
use crate::config::AppConfig;
pub fn spawn_ssh(_: &str, _: u16, _: &Credential) -> anyhow::Result<std::process::ExitStatus> { todo!() }
pub fn resolve_credential<'a>(_: &'a [Credential], _: &AppConfig, _: Option<&str>) -> anyhow::Result<Option<&'a Credential>> { Ok(None) }
pub fn connect_direct(_: &str) -> anyhow::Result<()> { todo!() }
