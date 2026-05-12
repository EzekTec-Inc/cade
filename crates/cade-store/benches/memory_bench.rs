use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use cade_store::sqlite;

fn bench_reciprocal_rank_fusion(c: &mut Criterion) {
    let kw_labels: Vec<String> = (0..100).map(|i| format!("label_{}", i)).collect();
    let sem_labels: Vec<String> = (50..150).map(|i| format!("label_{}", i)).collect();

    c.bench_function("reciprocal_rank_fusion", |b| {
        b.iter(|| {
            let result = sqlite::embedding::reciprocal_rank_fusion(
                black_box(&kw_labels),
                black_box(&sem_labels),
                black_box(60.0),
            );
            black_box(result);
        })
    });
}

criterion_group!(benches, bench_reciprocal_rank_fusion);
criterion_main!(benches);
