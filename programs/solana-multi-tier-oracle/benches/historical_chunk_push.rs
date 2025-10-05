use anchor_lang::prelude::Pubkey;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use solana_multi_tier_oracle::state::historical_chunk::{HistoricalChunk, PricePoint};
use solana_multi_tier_oracle::utils::constants::BUFFER_SIZE;

// Multiplier controlling how many times we overwrite the buffer during the
// sustained benchmark. A larger multiplier exercises wraparound and steady
// state behavior rather than one-off allocation costs.
const OVERWRITE_MULTIPLIER: usize = 16;

// Construct a deterministically zeroed chunk. Benchmarks must be reproducible
// and avoid incidental noise (random seeds, allocator state). Explicit
// construction mirrors the on-chain layout and makes it clear which fields are
// relevant to push performance.
fn empty_chunk() -> HistoricalChunk {
    HistoricalChunk {
        chunk_id: 0,
        head: 0,
        tail: 0,
        count: 0,
        creation_timestamp: 0,
        next_chunk: Pubkey::default(),
        oracle_state: Pubkey::default(),
        price_points: [PricePoint::default(); BUFFER_SIZE],
        bump: 0,
        reserved: [0u8; 511],
    }
}

// Deterministic price point generator used by benchmarks. Values are simple
// and avoid expensive arithmetic so the benchmark focuses on buffer push cost
// (index arithmetic, memory stores) rather than point construction overhead.
fn deterministic_price_point(seed: u64) -> PricePoint {
    PricePoint {
        price: seed as i128,
        volume: (seed.wrapping_mul(11)) as i128,
        conf: (seed % 1_000) as u64,
        timestamp: seed as i64,
    }
}

// Benchmark group measuring two complementary scenarios:
// 1) Filling an empty chunk from zero to capacity — captures the steady cost
//    of sequential writes before any wraparound logic activates.
// 2) Sustained overwrite/wraparound — exercises the rolling window behaviour
//    where tail advancement and masking happen frequently. This is the
//    production hot path under continuous updates and is the most important
//    scenario for measuring push throughput and memory/store throughput.
fn bench_historical_chunk_push(c: &mut Criterion) {
    let mut group = c.benchmark_group("historical_chunk_push");

    // Measure throughput in elements for the fill scenario.
    group.throughput(Throughput::Elements(BUFFER_SIZE as u64));
    group.bench_function("fill_empty_chunk", |b| {
        // Use `iter_batched` to provide a fresh chunk for each iteration and
        // isolate the operation under measurement from setup costs.
        b.iter_batched(
            empty_chunk,
            |mut chunk| {
                for idx in 0..BUFFER_SIZE as u64 {
                    chunk.push(deterministic_price_point(idx));
                }
                // `black_box` prevents the compiler from optimizing away
                // the measured operations.
                black_box(chunk)
            },
            BatchSize::SmallInput,
        );
    });

    // Sustained overwrite scenario: measure behavior when the buffer wraps
    // repeatedly. This stresses index masking and ensures the buffer's rolling
    // window semantics do not introduce pathological costs (e.g., hidden
    // reallocations or expensive branches).
    let total_writes = BUFFER_SIZE * OVERWRITE_MULTIPLIER;
    group.throughput(Throughput::Elements(total_writes as u64));
    group.bench_function("sustained_overwrite_wraparound", |b| {
        b.iter_batched(
            empty_chunk,
            |mut chunk| {
                for idx in 0..total_writes as u64 {
                    chunk.push(deterministic_price_point(idx));
                }
                black_box(chunk)
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_historical_chunk_push);
criterion_main!(benches);
