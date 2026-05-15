use cade_agent::mcp::McpManager;
use cade_agent::tools::manager::dispatch;
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn bench_tool_dispatch(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mcp = McpManager::empty();
    let args = serde_json::json!({});

    c.bench_function("dispatch_unknown_tool", |b| {
        b.to_async(&rt).iter(|| async {
            let res = dispatch(
                black_box("call_1".to_string()),
                black_box("unknown_tool_xyz"),
                black_box(&args),
                black_box(&mcp),
                black_box(None),
            )
            .await;
            black_box(res);
        })
    });
}

criterion_group!(benches, bench_tool_dispatch);
criterion_main!(benches);
