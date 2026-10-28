//! Jaeger (`uber-trace-id`) propagator, vendored from opentelemetry-jaeger-propagator 0.32.0
//! (Apache-2.0, https://github.com/open-telemetry/opentelemetry-rust). Upstream removed the
//! crate in 0.33; kept here so `propagator = "jaeger"` keeps working.
#![allow(dead_code, unexpected_cfgs)] // otel_warn! checks a cfg this crate does not define

use opentelemetry::{
    Context, otel_warn,
    propagation::{Extractor, Injector, TextMapPropagator, text_map_propagator::FieldIter},
    trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState},
};
use std::borrow::Cow;
use std::str::FromStr;

const JAEGER_HEADER: &str = "uber-trace-id";
const JAEGER_BAGGAGE_PREFIX: &str = "uberctx-";
const DEPRECATED_PARENT_SPAN: &str = "0";

const TRACE_FLAG_DEBUG: TraceFlags = TraceFlags::new(0x04);

/// `Propagator` implements the [Jaeger propagation format].
/// See [module level documentation](self#Examples) for examples.
///  
/// [jaeger propagation format]: https://www.jaegertracing.io/docs/1.18/client-libraries/#propagation-format
#[derive(Clone, Debug)]
pub struct Propagator {
    baggage_prefix: &'static str,
    header_name: &'static str,
    fields: [String; 1],
}

// Implement default using Propagator::new() to not break compatibility with previous versions
impl Default for Propagator {
    fn default() -> Self {
        Propagator::new()
    }
}

impl Propagator {
    /// Create a Jaeger propagator
    pub fn new() -> Self {
        Self::with_custom_header_and_baggage(JAEGER_HEADER, JAEGER_BAGGAGE_PREFIX)
    }

    /// Create a Jaeger propagator with custom header name
    pub fn with_custom_header(custom_header_name: &'static str) -> Self {
        Self::with_custom_header_and_baggage(custom_header_name, JAEGER_BAGGAGE_PREFIX)
    }

    /// Create a Jaeger propagator with custom header name and baggage prefix
    ///
    /// NOTE: it'll implicitly fallback to the default header names when the name of provided custom_* is empty
    /// Default header-name is `uber-trace-id` and baggage-prefix is `uberctx-`
    /// The format of serialized contexts and baggages stays unchanged and does not depend
    /// on provided header name and prefix.
    pub fn with_custom_header_and_baggage(
        custom_header_name: &'static str,
        custom_baggage_prefix: &'static str,
    ) -> Self {
        let custom_header_name = if custom_header_name.trim().is_empty() {
            JAEGER_HEADER
        } else {
            custom_header_name
        };

        let custom_baggage_prefix = if custom_baggage_prefix.trim().is_empty() {
            JAEGER_BAGGAGE_PREFIX
        } else {
            custom_baggage_prefix
        };

        Propagator {
            baggage_prefix: custom_baggage_prefix.trim(),
            header_name: custom_header_name.trim(),
            fields: [custom_header_name.to_owned()],
        }
    }

    /// Extract span context from header value
    fn extract_span_context(&self, extractor: &dyn Extractor) -> Option<SpanContext> {
        let mut header_value = Cow::from(extractor.get(self.header_name).unwrap_or(""));
        // if there is no :, it means header_value could be encoded as url, try decode first
        if !header_value.contains(':') {
            header_value = Cow::from(header_value.replace("%3A", ":"));
        }

        let parts = header_value.split_terminator(':').collect::<Vec<&str>>();
        if parts.len() != 4 {
            otel_warn!(
                name: "JaegerPropagator.InvalidHeader",
                message = "Invalid jaeger header format",
                header_value = header_value.to_string(),
            );
            return None;
        }

        match (
            self.extract_trace_id(parts[0]),
            self.extract_span_id(parts[1]),
            // Ignore parent span id since it's deprecated.
            self.extract_trace_flags(parts[3]),
            self.extract_trace_state(extractor),
        ) {
            (Ok(trace_id), Ok(span_id), Ok(flags), Ok(state)) => {
                Some(SpanContext::new(trace_id, span_id, flags, true, state))
            }
            _ => {
                otel_warn!(
                    name: "JaegerPropagator.InvalidHeader",
                    message = "Invalid jaeger header format",
                    header_value = header_value.to_string(),
                );
                None
            }
        }
    }

    /// Extract trace id from the header.
    fn extract_trace_id(&self, trace_id: &str) -> Result<TraceId, ()> {
        if trace_id.len() > 32 {
            return Err(());
        }

        TraceId::from_hex(trace_id).map_err(|_| ())
    }

    /// Extract span id from the header.
    fn extract_span_id(&self, span_id: &str) -> Result<SpanId, ()> {
        match span_id.len() {
            // exact 16
            16 => SpanId::from_hex(span_id).map_err(|_| ()),
            // more than 16 is invalid
            17.. => Err(()),
            // less than 16 will result in padding on left
            _ => {
                let padded = format!("{span_id:0>16}");
                SpanId::from_hex(&padded).map_err(|_| ())
            }
        }
    }

    /// Extract flag from the header
    ///
    /// First bit controls whether to sample
    /// Second bit controls whether it's a debug trace
    /// Third bit is not used.
    /// Fourth bit is firehose flag, which is not supported in OT now.
    fn extract_trace_flags(&self, flag: &str) -> Result<TraceFlags, ()> {
        if flag.len() > 2 {
            return Err(());
        }
        let flag = u8::from_str(flag).map_err(|_| ())?;
        if flag & 0x01 == 0x01 {
            if flag & 0x02 == 0x02 {
                Ok(TraceFlags::SAMPLED | TRACE_FLAG_DEBUG)
            } else {
                Ok(TraceFlags::SAMPLED)
            }
        } else {
            // Debug flag should only be set when sampled flag is set.
            // So if debug flag is set alone. We will just use not sampled flag
            Ok(TraceFlags::default())
        }
    }

    fn extract_trace_state(&self, extractor: &dyn Extractor) -> Result<TraceState, ()> {
        let baggage_keys = extractor
            .keys()
            .into_iter()
            .filter(|key| key.starts_with(self.baggage_prefix))
            .filter_map(|key| {
                extractor
                    .get(key)
                    .map(|value| (key.to_string(), value.to_string()))
            });

        match TraceState::from_key_value(baggage_keys) {
            Ok(trace_state) => Ok(trace_state),
            Err(trace_state_err) => {
                otel_warn!(
                    name: "JaegerPropagator.InvalidTraceState",
                    message = "Invalid trace state",
                    reason = format!("{:?}", trace_state_err),
                );
                Err(()) //todo: assign an error type instead of using ()
            }
        }
    }
}

impl TextMapPropagator for Propagator {
    fn inject_context(&self, cx: &Context, injector: &mut dyn Injector) {
        let span = cx.span();
        let span_context = span.span_context();
        if span_context.is_valid() {
            let flag: u8 = if span_context.is_sampled() {
                if span_context.trace_flags() & TRACE_FLAG_DEBUG == TRACE_FLAG_DEBUG {
                    0x03
                } else {
                    0x01
                }
            } else {
                0x00
            };
            let header_value = format!(
                "{}:{}:{:01}:{:01x}",
                span_context.trace_id(),
                span_context.span_id(),
                DEPRECATED_PARENT_SPAN,
                flag,
            );
            injector.set(self.header_name, header_value);
        }
    }

    fn extract_with_context(&self, cx: &Context, extractor: &dyn Extractor) -> Context {
        self.extract_span_context(extractor)
            .map(|sc| cx.with_remote_span_context(sc))
            .unwrap_or_else(|| cx.clone())
    }

    fn fields(&self) -> FieldIter<'_> {
        FieldIter::new(self.fields.as_ref())
    }
}
