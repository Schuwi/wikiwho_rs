use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::{Rng, SeedableRng};
use wikiwho_rs::utils;

fn generate_input_split_into_paragraphs(length: u64) -> String {
    // generate inputs from fixed seeds
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(length); /* define specific algorithm to ensure reproducibility */
    let mut input = String::new();
    for _ in 0..length {
        input.push(rng.gen());
    }

    // add some expected values at random places
    const VALUES: &[&str] = &[
        "\r", "\n", "\r\n", "\n\n", "{|", "|}", "|-\n", "<table>", "</table>", "<tr>", "</tr>",
    ];
    for _ in 0..(length / 10) {
        let mut pos = rng.gen_range(0..input.len());
        while !input.is_char_boundary(pos) {
            pos = rng.gen_range(0..input.len());
        }

        let value = VALUES[rng.gen_range(0..VALUES.len())];
        input.insert_str(pos, value);
    }

    input
}

fn bench_split_into_paragraphs(c: &mut Criterion) {
    let mut group = c.benchmark_group("split_into_paragraphs");
    for length in [500u64, 1000u64, 5000u64, 10000u64].into_iter() {
        let input = generate_input_split_into_paragraphs(length);
        group.bench_with_input(BenchmarkId::new("Naive", length), &input, |b, i| {
            b.iter(|| utils::split_into_paragraphs_naive(i));
        });
        group.bench_with_input(BenchmarkId::new("Optimized", length), &input, |b, i| {
            let mut scratch_buffers = (String::new(), String::new());
            b.iter(|| {
                utils::split_into_paragraphs_optimized(
                    i,
                    (&mut scratch_buffers.0, &mut scratch_buffers.1),
                )
            });
        });
    }
}

fn generate_input_split_into_sentences(length: u64) -> String {
    // generate inputs from fixed seeds
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(length); /* define specific algorithm to ensure reproducibility */
    let mut input = String::new();
    for _ in 0..length {
        input.push(rng.gen());
    }

    // add some expected values at random places
    const VALUES: &[&str] = &[
        " ", "\n", ". ", ", ", "; ", ": ", "? ", "! ", "//", "http", "<!--", "-->", "<ref", "/ref>",
    ];
    for _ in 0..(length / 10) {
        let mut pos = rng.gen_range(0..input.len());
        while !input.is_char_boundary(pos) {
            pos = rng.gen_range(0..input.len());
        }

        let value = VALUES[rng.gen_range(0..VALUES.len())];
        input.insert_str(pos, value);
    }

    input
}

fn bench_split_into_sentences(c: &mut Criterion) {
    let mut group = c.benchmark_group("split_into_sentences");
    for length in [100u64, 500u64, 1000u64, 5000u64].into_iter() {
        let input = generate_input_split_into_sentences(length);
        group.bench_with_input(BenchmarkId::new("Naive", length), &input, |b, i| {
            b.iter(|| utils::split_into_sentences_naive(i));
        });
        group.bench_with_input(BenchmarkId::new("Optimized", length), &input, |b, i| {
            let mut scratch_buffers = (String::new(), String::new());
            b.iter(|| {
                utils::split_into_sentences_optimized(
                    i,
                    (&mut scratch_buffers.0, &mut scratch_buffers.1),
                )
            });
        });
    }
}

fn generate_input_split_into_tokens(length: u64) -> String {
    // generate inputs from fixed seeds
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(length); /* define specific algorithm to ensure reproducibility */
    let mut input = String::new();
    for _ in 0..length {
        input.push(rng.gen());
    }

    // add some expected values at random places
    const VALUES: &[&str] = &[
        " ", "\n", "<!--", "-->", "[[", "]]", "{{", "}}", "|", ".", ",", ";", ":", "?", "!", "-",
        "_", "/", "\\", "(", ")", "[", "]", "{", "}", "*", "#", "@", "&", "=", "+", "%", "~", "$",
        "^", "<", ">", "\"", "'", "´", "`", "¸", "˛", "’", "¤", "₳", "฿", "₵", "¢", "₡", "₢", "₫",
        "₯", "֏", "₠", "€", "ƒ", "₣", "₲", "₴", "₭", "₺", "₾", "ℳ", "₥", "₦", "₧", "₱", "₰", "£",
        "៛", "₽", "₹", "₨", "₪", "৳", "₸", "₮", "₩", "¥", "§", "‖", "¦", "⟨", "⟩", "–", "—", "¯",
        "»", "«", "”", "÷", "×", "′", "″", "‴", "¡", "¿", "©", "℗", "®", "℠", "™",
    ];
    for _ in 0..(length / 10) {
        let mut pos = rng.gen_range(0..input.len());
        while !input.is_char_boundary(pos) {
            pos = rng.gen_range(0..input.len());
        }

        let value = VALUES[rng.gen_range(0..VALUES.len())];
        input.insert_str(pos, value);
    }

    input
}

fn bench_split_into_tokens(c: &mut Criterion) {
    let mut group = c.benchmark_group("split_into_tokens");
    for length in [10u64, 50u64, 100u64, 500u64].into_iter() {
        let input = generate_input_split_into_tokens(length);
        group.bench_with_input(BenchmarkId::new("Naive", length), &input, |b, i| {
            b.iter(|| utils::split_into_tokens_naive(i));
        });
        group.bench_with_input(BenchmarkId::new("Corasick", length), &input, |b, i| {
            b.iter(|| utils::split_into_tokens_corasick(i));
        });
    }
}

criterion_group!(
    benches,
    bench_split_into_paragraphs,
    bench_split_into_sentences,
    bench_split_into_tokens
);
criterion_main!(benches);
