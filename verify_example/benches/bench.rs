#![feature(test)]

extern crate test;

use test::Bencher;

#[bench]
fn bench_load_key(b: &mut Bencher) {
    b.iter(|| verify_example::load_key(verify_example::ROOT_KEY.as_bytes()).unwrap());
}

#[bench]
fn bench_verify_merkle_root(b: &mut Bencher) {
    let key = verify_example::load_key(verify_example::ROOT_KEY.as_bytes()).unwrap();
    b.iter(|| verify_example::verify(verify_example::ROOT_5360668_PGP.as_bytes(), &key).unwrap());
}

#[bench]
fn bench_parse_root_json(b: &mut Bencher) {
    b.iter(|| {
        let v: serde_json::Value = serde_json::from_str(verify_example::ROOT_5360668_JSON).unwrap();
        assert!(v.get("body").is_some());
    });
}
