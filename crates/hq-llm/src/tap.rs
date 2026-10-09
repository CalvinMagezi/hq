//! Records one outcome per streamed call, when the stream ends, so the ledger carries the usage the
//! provider reports in the final chunks instead of the empty row the first chunk used to produce.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio_stream::Stream;

use crate::cost::{ProviderClass, Usage};
use crate::outcome_sink::{SessionContext, SharedSink};
use crate::provider::StreamChunk;

use crate::instrument::{OutcomeInput, emit_outcome};
use crate::router::TaskHint;

pub(crate) type ChunkStream = Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>;

struct Pending {
    sink: SharedSink,
    ctx: SessionContext,
    provider: String,
    class: ProviderClass,
    model: String,
    task: TaskHint,
    started: Instant,
    latency: Option<Duration>,
    usage: Option<Usage>,
    resolved_model: Option<String>,
    failure: Option<String>,
}

impl Pending {
    fn observe(&mut self, item: &Result<StreamChunk>) {
        match item {
            Ok(StreamChunk::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            }) => {
                let kept = self.usage.unwrap_or_default();
                self.usage = Some(Usage {
                    input: *input_tokens,
                    output: *output_tokens,
                    cache_read: *cache_read_tokens,
                    cache_write: *cache_write_tokens,
                    ..kept
                });
            }
            Ok(StreamChunk::Billing {
                cost_usd,
                reasoning_tokens,
            }) => {
                let usage = self.usage.get_or_insert_with(Usage::default);
                usage.billed_usd = *cost_usd;
                usage.reasoning = *reasoning_tokens;
            }
            Ok(StreamChunk::ModelInfo(m)) if !m.is_empty() => {
                self.resolved_model = Some(m.clone());
            }
            Err(e) => self.failure = Some(e.to_string()),
            _ => {}
        }
    }

    fn finish(self, end: End) {
        let error = match end {
            End::Errored => Some("stream_error"),
            End::EndedWithoutDone => Some("truncated"),
            End::Done | End::Dropped => None,
        };
        emit_outcome(
            &self.sink,
            self.ctx,
            OutcomeInput {
                provider: &self.provider,
                class: self.class,
                model: self.resolved_model.as_deref().unwrap_or(&self.model),
                task: self.task,
                latency: self.latency.unwrap_or_else(|| self.started.elapsed()),
                usage: self.usage,
                error,
                cancelled: end == End::Dropped,
            },
        );
    }
}

/// How a streamed call ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    Done,
    Errored,
    EndedWithoutDone,
    /// The reader walked away before the stream finished.
    Dropped,
}

pub(crate) struct OutcomeTap {
    inner: ChunkStream,
    pending: Option<Pending>,
}

impl OutcomeTap {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn wrap(
        inner: ChunkStream,
        sink: SharedSink,
        ctx: SessionContext,
        provider: &str,
        class: ProviderClass,
        model: &str,
        task: TaskHint,
        started: Instant,
    ) -> ChunkStream {
        Box::pin(Self {
            inner,
            pending: Some(Pending {
                sink,
                ctx,
                provider: provider.to_string(),
                class,
                model: model.to_string(),
                task,
                started,
                latency: None,
                usage: None,
                resolved_model: None,
                failure: None,
            }),
        })
    }
}

impl Stream for OutcomeTap {
    type Item = Result<StreamChunk>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let polled = self.inner.as_mut().poll_next(cx);
        match &polled {
            Poll::Ready(Some(item)) => {
                if let Some(p) = self.pending.as_mut() {
                    let started = p.started;
                    p.latency.get_or_insert_with(|| started.elapsed());
                    p.observe(item);
                }
                if matches!(item, Ok(StreamChunk::Done) | Err(_))
                    && let Some(p) = self.pending.take()
                {
                    p.finish(if item.is_ok() {
                        End::Done
                    } else {
                        End::Errored
                    });
                }
            }
            Poll::Ready(None) => {
                if let Some(p) = self.pending.take() {
                    p.finish(End::EndedWithoutDone);
                }
            }
            Poll::Pending => {}
        }
        polled
    }
}

impl Drop for OutcomeTap {
    fn drop(&mut self) {
        if let Some(p) = self.pending.take() {
            p.finish(End::Dropped);
        }
    }
}
