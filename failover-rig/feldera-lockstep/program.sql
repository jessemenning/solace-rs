-- Minimal lockstep program (§12 task 5, Capability-2 warm-replica proof).
-- Two Feldera replicas each run this, differing only in the input queue (__FQ__).
-- Both queues subscribe to `failover/in/>`, so both replicas ingest the SAME keyed
-- stream and should maintain an IDENTICAL `rolling` view — the warm-redundancy
-- foundation HA rests on. The view is read back over REST /query (no solace_output,
-- so no changelog-retraction handling needed).

-- topic_pattern captures are exposed via CONNECTOR_METADATA() and must be pulled into a
-- column with a DEFAULT CAST (they are NOT name-matched automatically). `k` is the key
-- level of the topic `failover/in/{k}`.
CREATE TABLE inp (
    seq VARCHAR,          -- raw payload (the sequence number)
    k   VARCHAR DEFAULT CAST(CONNECTOR_METADATA()['k'] AS VARCHAR)
) WITH (
  'connectors' = '[{
    "transport": {
      "name": "solace_input",
      "config": {
        "host": "solace",
        "port": 55555,
        "vpn": "default",
        "username": "default",
        "password": "default",
        "queue": "__FQ__",
        "topic_pattern": "failover/in/{k}"
      }
    },
    "format": {"name": "raw", "config": {"column_name": "seq"}}
  }]'
);

-- Per-key rolling count: the running aggregate both replicas must agree on.
CREATE MATERIALIZED VIEW rolling AS
SELECT k, COUNT(*) AS cnt
FROM inp
GROUP BY k;
