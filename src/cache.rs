use std::sync::OnceLock;

use haproxy_api::Txn;
use mlua::prelude::LuaString;
use opentelemetry::trace::TraceContextExt as _;
use opentelemetry::{Context, TraceId};

// This is a global cache to store the context of the spans
// It can be reused independently of http session in many listeners
static TRACE_CACHE: OnceLock<quick_cache::sync::Cache<[u8; 16], Context>> = OnceLock::new();

fn init_cache() -> quick_cache::sync::Cache<[u8; 16], Context> {
    quick_cache::sync::Cache::new(100_000)
}

// Get the context from the global cache
pub(crate) fn get_context(txn: &Txn) -> Option<Context> {
    let trace_id = txn.get_var::<LuaString>("txn.otel_trace_id").ok()?;
    let mut trace_bytes = [0u8; 16];
    const_hex::decode_to_slice(trace_id.as_bytes(), &mut trace_bytes).ok()?;
    TRACE_CACHE.get_or_init(init_cache).get(&trace_bytes)
}

// Store the context in the globally cache to share it between listeners/frontends
pub(crate) fn store_context(txn: &Txn, trace_id: TraceId, context: Context) {
    let trace_id_bytes = trace_id.to_bytes();
    let trace_id_hex = const_hex::encode(trace_id_bytes);
    let span_id_hex = const_hex::encode(context.span().span_context().span_id().to_bytes());
    let _ = txn.set_var("txn.otel_trace_id", &*trace_id_hex);
    let _ = txn.set_var("txn.otel_span_id", &*span_id_hex);
    TRACE_CACHE
        .get_or_init(init_cache)
        .insert(trace_id_bytes, context);
}

pub(crate) fn remove_context(txn: &Txn) -> Option<Context> {
    let trace_id = txn.get_var::<LuaString>("txn.otel_trace_id").ok()?;
    let mut trace_bytes = [0u8; 16];
    const_hex::decode_to_slice(trace_id.as_bytes(), &mut trace_bytes).ok()?;
    TRACE_CACHE
        .get_or_init(init_cache)
        .remove(&trace_bytes)
        .map(|(_, context)| context)
}

pub(crate) fn get_size() -> usize {
    TRACE_CACHE.get().map(|c| c.len()).unwrap_or(0)
}
