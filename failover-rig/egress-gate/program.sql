-- Egress-gate program (§12 task 5, end-to-end Capability 2).
-- Same warm-replica count as feldera-lockstep, plus a solace_output sink so each replica
-- PUBLISHES its `rolling` view. The egress gate relays only the leader's output downstream.
-- Parameterised per replica: __FQ__ (input queue), __FOUT__ (output topic base).

CREATE TABLE inp (
    seq VARCHAR,
    k   VARCHAR DEFAULT CAST(CONNECTOR_METADATA()['k'] AS VARCHAR)
) WITH (
  'connectors' = '[{
    "transport": {
      "name": "solace_input",
      "config": {
        "host": "solace", "port": 55555, "vpn": "default",
        "username": "default", "password": "default",
        "queue": "__FQ__",
        "topic_pattern": "failover/in/{k}"
      }
    },
    "format": {"name": "raw", "config": {"column_name": "seq"}}
  }]'
);

CREATE MATERIALIZED VIEW rolling AS
SELECT k, COUNT(*) AS cnt FROM inp GROUP BY k;

-- Publish the rolling view. One topic per key; dedup_window conflates each key's rapid
-- changelog updates to the latest (k,cnt) payload so the gate/recorder see latest-value,
-- not every intermediate retraction.
CREATE MATERIALIZED VIEW rolling_sink
WITH ('connectors' = '[{
    "transport": {
        "name": "solace_output",
        "config": {
            "host": "solace", "port": 55555, "vpn": "default",
            "username": "default", "password": "default",
            "topic": "__FOUT__/{k}",
            "delivery_mode": "direct",
            "dedup_window_ms": 300
        }
    },
    "format": {"name": "json"}
}]')
AS SELECT k, cnt FROM rolling;
