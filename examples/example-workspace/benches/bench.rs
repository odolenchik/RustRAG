use criterion::{black_box, criterion_group, criterion_main, Criterion};
use example_workspace::utils::{is_palindrome, to_title_case};

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("to_title_case", |b| {
        b.iter(|| {
            to_title_case(black_box("hello world rust rag example"))
        })
    });
    
    c.bench_function("is_palindrome", |b| {
        b.iter(|| {
            is_palindrome(black_box("A man, a plan, a canal: Panama"))
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
