use criterion::{black_box, criterion_group, criterion_main, Criterion};
use iced::futures::executor::block_on;
use local_ip_address::local_ip;
use std::thread;

fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 1,
        1 => 1,
        n => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn bench_local_transfer(c: &mut Criterion) {
    // Start server in one thread
    thread::spawn(move || {
        let my_local_ip = local_ip().unwrap();
        let port = 8888;
        // let server = Server::init(my_local_ip.to_string(), port);
        // block_on(server.run());
        // Result::Ok(())
    });

    c.bench_function("onefile", |b| b.iter(|| fibonacci(black_box(20))));
}

criterion_group!(benches, bench_local_transfer);
criterion_main!(benches);
