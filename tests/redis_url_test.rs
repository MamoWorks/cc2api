//! Regression tests for Redis connection URL construction.
//!
//! Previously `RedisStore::new` concatenated `addr` (which already contained
//! `/{db}`) with `/{db}` again, producing URLs like `redis://host:6379/0/0`
//! that the `redis` crate rejects with `InvalidClientConfig`.
//!
//! See issue #6 / `fix: correct Redis URL construction (double /db)`.

use claude_code_gateway::store::redis::build_redis_url;

#[test]
fn build_url_without_password() {
    let url = build_redis_url("127.0.0.1", 6379, "", 0);
    assert_eq!(url, "redis://127.0.0.1:6379/0");
}

#[test]
fn build_url_with_password() {
    let url = build_redis_url("127.0.0.1", 6379, "secret", 0);
    assert_eq!(url, "redis://:secret@127.0.0.1:6379/0");
}

#[test]
fn build_url_with_non_zero_db() {
    let url = build_redis_url("redis.internal", 6380, "", 3);
    assert_eq!(url, "redis://redis.internal:6380/3");
}

#[test]
fn build_url_with_password_and_non_zero_db() {
    let url = build_redis_url("redis.internal", 6380, "p@ss", 7);
    assert_eq!(url, "redis://:p@ss@redis.internal:6380/7");
}

/// Critical regression: the db path segment must appear exactly once.
/// The old buggy code produced `.../0/0` which `redis::Client::open` rejects
/// as `Invalid database number- InvalidClientConfig`.
#[test]
fn url_contains_db_exactly_once() {
    let cases = [
        build_redis_url("host", 6379, "", 0),
        build_redis_url("host", 6379, "pw", 0),
        build_redis_url("host", 6379, "", 5),
        build_redis_url("host", 6379, "pw", 5),
    ];
    for url in cases {
        // Strip scheme, then count path slashes. There must be exactly one:
        // the slash preceding the db segment.
        let after_scheme = url.strip_prefix("redis://").expect("redis:// scheme");
        let slashes: usize = after_scheme.chars().filter(|c| *c == '/').count();
        assert_eq!(
            slashes, 1,
            "expected exactly one '/' after the authority in {url}"
        );
    }
}

/// The constructed URL must be accepted by `redis::Client::open`. This is the
/// narrowest possible live-parser check — it does not connect to a server,
/// it only exercises URL parsing, which is what failed before the fix.
#[test]
fn parsed_by_redis_client() {
    let url = build_redis_url("127.0.0.1", 6379, "", 0);
    redis::Client::open(url).expect("redis client should accept the URL");

    let url = build_redis_url("127.0.0.1", 6379, "secret", 2);
    redis::Client::open(url).expect("redis client should accept URL with password and db");
}
