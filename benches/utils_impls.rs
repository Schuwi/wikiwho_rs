use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::{Rng, SeedableRng};
use wikiwho_rs::utils;

fn generate_input_split_into_paragraphs(length: u64) -> String {
    // generate inputs from fixed seeds
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(length); /* define specific algorithm to ensure reproducibility */
    let mut input = String::new();
    for _ in 0..length {
        input.push(rng.gen_range(0..128) as u8 as char);
    }

    // add some expected values at random places
    const VALUES: [&str; 17] = [
        "\n", "\n\n", "\n\n\n", "\r\n", "\r", "\r\r", "\r\r\r", "\r\n\r\n", "\n\r\n", "\n\n\r",
        "<table>", "</table>", "<tr>", "</tr>", "{|", "|}", "|-\n",
    ];
    for _ in 0..(length / 10) {
        let pos = rng.gen_range(0..input.len());
        input.insert_str(pos, VALUES[rng.gen_range(0..VALUES.len())]);
    }

    input
}

fn bench_split_into_paragraphs(c: &mut Criterion) {
    let mut group = c.benchmark_group("split_into_paragraphs");
    for length in [100u64, 1000u64, 10000u64, 100000u64].into_iter() {
        let input = generate_input_split_into_paragraphs(length);
        group.bench_with_input(BenchmarkId::new("Naive", length), &input, |b, i| {
            b.iter(|| utils::split_into_paragraphs_naive(i));
        });
        group.bench_with_input(BenchmarkId::new("Corasick", length), &input, |b, i| {
            b.iter(|| utils::split_into_paragraphs_corasick(i));
        });
    }
}

criterion_group!(benches, bench_split_into_paragraphs);
criterion_main!(benches);
