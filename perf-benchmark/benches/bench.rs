use std::{str::FromStr, sync::atomic::AtomicBool};

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use example_protobuf::{health_client::HealthClient, WrappedClient};
use futures::future::try_join_all;
use soda_pool::EndpointTemplate;
use std::sync::atomic::Ordering::Relaxed;
use tokio::runtime::Runtime;
use tonic::{transport::Endpoint, IntoRequest, Status};
use url::Url;

pub fn grpc_client(c: &mut Criterion) {
    let runner = Runtime::new().unwrap();

    let mut group = c.benchmark_group("grpc_client");

    let address = std::env::var("ADDRESS").unwrap_or_else(|_| "http://localhost:50001".to_string());

    let template = EndpointTemplate::new(Url::parse(&address).unwrap()).unwrap();
    let endpoint = Endpoint::from_str(address.as_str()).unwrap();
    let client = WrappedClient::new(template);

    let test_cases = [1, 2, 4, 8, 16, 32, 64];
    let prev_test_failed = AtomicBool::new(false);

    for i in test_cases.iter() {
        if prev_test_failed.load(Relaxed) {
            break;
        }
        group.throughput(Throughput::Elements(*i as u64));
        group.bench_with_input(BenchmarkId::new("wrapped", i), &i, |b, _i| {
            b.to_async(&runner).iter(|| async {
                let res = try_join_all((0..*i).map(|_| client.is_alive(()))).await;
                prev_test_failed.store(black_box(res).is_err(), Relaxed);
            })
        });
    }
    if prev_test_failed.load(Relaxed) {
        println!("Some tests failed.");
    }

    drop(client);

    prev_test_failed.store(false, Relaxed);
    for i in test_cases.iter() {
        if prev_test_failed.load(Relaxed) {
            break;
        }
        group.throughput(Throughput::Elements(*i as u64));
        group.bench_with_input(BenchmarkId::new("reconnect", i), &i, |b, _i| {
            b.to_async(&runner).iter(|| async {
                let res = try_join_all((0..*i).map(|_| async {
                    let mut client = HealthClient::connect(endpoint.clone())
                        .await
                        .map_err(|_| Status::unknown(""))?;
                    client.is_alive(().into_request()).await
                }))
                .await;
                prev_test_failed.store(black_box(res).is_err(), Relaxed);
            })
        });
    }
    if prev_test_failed.load(Relaxed) {
        println!("Some tests failed.");
    }
}

fn grpc_connection(c: &mut Criterion) {
    let runner = Runtime::new().unwrap();

    let mut group = c.benchmark_group("grpc_connection");

    let address = std::env::var("ADDRESS").unwrap_or_else(|_| "http://localhost:50001".to_string());

    let endpoint = Endpoint::from_str(address.as_str()).unwrap();

    group.bench_function("connect", |b| {
        b.to_async(&runner).iter(|| async {
            let _ = black_box(HealthClient::connect(endpoint.clone()).await);
        });
    });
}

criterion_group!(benches, grpc_client, grpc_connection);
criterion_main!(benches);
