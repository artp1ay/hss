use crate::types::{Host, ServerRecord};
pub fn parse_inventory(_: &str) -> Vec<Host> { vec![] }
pub fn sync_server_records(r: Vec<ServerRecord>, _: &[String]) -> Vec<ServerRecord> { r }
pub fn load_and_sync(_: &str) -> anyhow::Result<(Vec<Host>, Vec<ServerRecord>)> { Ok((vec![], vec![])) }
