use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use styx_proto::decode;

fn synthetic_blob(target_bytes: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(target_bytes + 128);
    output.push(b'd');
    let mut index = 0usize;

    while output.len() < target_bytes {
        let key = format!("k{index:08}");
        let value = format!("value-{index:08}");
        output.extend_from_slice(key.len().to_string().as_bytes());
        output.push(b':');
        output.extend_from_slice(key.as_bytes());
        output.extend_from_slice(value.len().to_string().as_bytes());
        output.push(b':');
        output.extend_from_slice(value.as_bytes());
        index += 1;
    }

    output.push(b'e');
    output
}

fn bench_decode_1mb(c: &mut Criterion) {
    let input = synthetic_blob(1024 * 1024);
    let mut group = c.benchmark_group("bencode_decode");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function("decode_1mb_dictionary", |b| {
        b.iter(|| decode(black_box(&input)).expect("synthetic bencode remains valid"));
    });
    group.finish();
}

criterion_group!(benches, bench_decode_1mb);
criterion_main!(benches);
