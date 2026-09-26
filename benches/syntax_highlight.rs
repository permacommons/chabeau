use chabeau::ui::theme::Theme;
use chabeau::utils::syntax::highlight_code_block;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

const RUST: &str = r#"use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Session {
    id: u64,
    name: String,
    tags: Vec<String>,
}

impl Session {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self { id, name: name.into(), tags: Vec::new() }
    }

    pub fn tag_counts(sessions: &[Session]) -> HashMap<&str, usize> {
        let mut counts = HashMap::new();
        for s in sessions {
            for t in &s.tags {
                *counts.entry(t.as_str()).or_insert(0) += 1;
            }
        }
        counts
    }
}

fn main() {
    let s = Session::new(42, "demo");
    println!("{:?} {}", s, s.id); // trailing comment
}
"#;

const PYTHON: &str = r#"import asyncio
from dataclasses import dataclass, field


@dataclass
class Message:
    role: str
    content: str
    tokens: list[int] = field(default_factory=list)


async def stream(messages: list[Message], limit: int = 10) -> str:
    """Join message contents, pausing between chunks."""
    out = []
    for i, m in enumerate(messages[:limit]):
        if m.role == "system":
            continue
        out.append(f"{i}: {m.content!r}")
        await asyncio.sleep(0)
    return "\n".join(out)


if __name__ == "__main__":
    msgs = [Message("user", "hi"), Message("assistant", "hello")]
    print(asyncio.run(stream(msgs)))
"#;

const JAVASCRIPT: &str = r#"const API_URL = "https://example.invalid/v1/chat";

export async function send(messages, { model = "default", signal } = {}) {
  const res = await fetch(API_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ model, messages, stream: true }),
    signal,
  });
  if (!res.ok) {
    throw new Error(`request failed: ${res.status}`);
  }
  const reader = res.body.getReader();
  let text = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    text += new TextDecoder().decode(value); // accumulate
  }
  return text.split("\n").filter((l) => l.startsWith("data:"));
}
"#;

fn bench_load(c: &mut Criterion) {
    let mut g = c.benchmark_group("syntax_load");
    g.sample_size(20);
    g.bench_function("syntax_set_defaults_newlines", |b| {
        b.iter(|| black_box(syntect::parsing::SyntaxSet::load_defaults_newlines()))
    });
    g.finish();
}

fn bench_highlight(c: &mut Criterion) {
    let theme = Theme::dark_default();
    // Warm the lazily loaded syntax and theme sets outside the measurement.
    let _ = highlight_code_block("rust", "fn main() {}", &theme);

    // A long block approximates re-highlighting a large fence mid-stream.
    let rust_long = RUST.repeat(6);

    let mut g = c.benchmark_group("syntax_highlight");
    for (id, lang, code) in [
        ("rust", "rust", RUST),
        ("rust_long", "rust", rust_long.as_str()),
        ("python", "python", PYTHON),
        ("javascript", "javascript", JAVASCRIPT),
    ] {
        g.throughput(Throughput::Bytes(code.len() as u64));
        let mut n: u64 = 0;
        g.bench_with_input(BenchmarkId::from_parameter(id), &code, |b, code| {
            b.iter_batched(
                || {
                    // A unique trailing line defeats the highlight cache.
                    n += 1;
                    format!("{code}\n{n}\n")
                },
                |src| black_box(highlight_code_block(lang, &src, &theme)),
                BatchSize::SmallInput,
            )
        });
    }
    g.finish();
}

criterion_group!(benches, bench_load, bench_highlight);
criterion_main!(benches);
