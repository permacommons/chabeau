use chabeau::api::adapters::AdapterKind;
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

fn decode_provider_streams(c: &mut Criterion) {
    let openai = include_str!("../src/api/fixtures/openai_stream.jsonl");
    let anthropic = include_str!("../src/api/fixtures/anthropic_stream.jsonl");
    c.bench_function("decode openai stream events", |b| {
        b.iter(|| {
            for line in black_box(openai).lines() {
                black_box(
                    AdapterKind::OpenaiChatCompletions
                        .adapter()
                        .decode_event(line),
                );
            }
        })
    });
    c.bench_function("decode anthropic stream events", |b| {
        b.iter(|| {
            for line in black_box(anthropic).lines() {
                black_box(AdapterKind::AnthropicMessages.adapter().decode_event(line));
            }
        })
    });
}

criterion_group!(benches, decode_provider_streams);
criterion_main!(benches);
