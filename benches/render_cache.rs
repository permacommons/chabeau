use chabeau::core::app::App;
use chabeau::core::message::Message;
use chabeau::ui::theme::Theme;
use chabeau::utils::scroll::ScrollCalculator;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::VecDeque;

fn make_messages(n_pairs: usize, base: &str) -> VecDeque<Message> {
    let mut v = VecDeque::new();
    for i in 0..n_pairs {
        v.push_back(Message {
            role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
            content: base.into(),
        });
        v.push_back(Message {
            role: if i % 2 == 0 { "assistant" } else { "user" }.into(),
            content: base.into(),
        });
    }
    v
}

fn redraw_no_cache(
    messages: &VecDeque<Message>,
    theme: &Theme,
    markdown: bool,
    syntax: bool,
    width: u16,
) {
    let built = ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
        messages, theme, markdown, syntax, None,
    );
    let _pre = ScrollCalculator::prewrap_lines(&built, width);
}

fn redraw_with_cache(app: &mut App, width: u16) {
    let _ = app.get_prewrapped_lines_cached(width);
}

fn bench_render_cache(c: &mut Criterion) {
    let base = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua";
    let theme = Theme::dark_default();
    let markdown = true;
    let syntax = false; // cheaper to keep parsing overhead consistent
    let width_small = 80u16;
    let width_large = 120u16;

    for &pairs in &[100usize, 400usize] {
        // ~200 and ~800 messages
        let messages = make_messages(pairs, base);
        let mut app = App::new_bench(theme.clone(), markdown, syntax);
        app.ui.messages = messages.clone();

        let built = ScrollCalculator::build_display_lines_with_theme_and_flags_and_width(
            &messages, &theme, markdown, syntax, None,
        );
        let logical_len = built.len();

        let mut group = c.benchmark_group(format!("render_cache_pairs{}", pairs));
        group.throughput(Throughput::Elements(logical_len as u64));

        group.bench_function(BenchmarkId::new("no_cache", width_small), |b| {
            b.iter(|| redraw_no_cache(&messages, &theme, markdown, syntax, width_small))
        });
        group.bench_function(BenchmarkId::new("with_cache", width_small), |b| {
            b.iter(|| redraw_with_cache(&mut app, width_small))
        });

        // Also test a different width (forces rebuild once, then reuse)
        group.bench_function(BenchmarkId::new("with_cache", width_large), |b| {
            b.iter(|| redraw_with_cache(&mut app, width_large))
        });

        // Streaming-like scenario: incrementally append to last message
        let mut messages_stream = messages.clone();
        if let Some(last) = messages_stream.back_mut() {
            last.content.push_str(" start");
        }
        let mut app_stream = App::new_bench(theme.clone(), markdown, syntax);
        app_stream.ui.messages = messages_stream.clone();

        group.bench_function(BenchmarkId::new("no_cache_stream", width_small), |b| {
            b.iter(|| {
                if let Some(last) = messages_stream.back_mut() {
                    last.content.push('.');
                }
                redraw_no_cache(&messages_stream, &theme, markdown, syntax, width_small)
            })
        });
        group.bench_function(BenchmarkId::new("with_cache_stream", width_small), |b| {
            b.iter(|| {
                if let Some(last) = app_stream.ui.messages.back_mut() {
                    last.content.push('.');
                }
                redraw_with_cache(&mut app_stream, width_small)
            })
        });

        group.finish();
    }
}

criterion_group!(benches, bench_render_cache);
criterion_main!(benches);
