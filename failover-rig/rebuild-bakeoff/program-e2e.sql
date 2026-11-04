-- End-to-end catch-up target: a cold Feldera that rebuilds `rolling` by ingesting the
-- re-fed backlog on bakeoff/feed/{k}. The re-feeder drains the source (replay or shadow)
-- and republishes here; catch-up = time until SUM(cnt) reaches the backlog size.

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
        "queue": "bakeoff-feld-q",
        "topic_pattern": "bakeoff/feed/{k}"
      }
    },
    "format": {"name": "raw", "config": {"column_name": "seq"}}
  }]'
);

CREATE MATERIALIZED VIEW rolling AS
SELECT k, COUNT(*) AS cnt FROM inp GROUP BY k;
