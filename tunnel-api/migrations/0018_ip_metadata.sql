CREATE TABLE ip_metadata (
  address TEXT PRIMARY KEY,
  details_json TEXT CHECK(details_json IS NULL OR json_valid(details_json)),
  refresh_after INTEGER NOT NULL
) STRICT;

CREATE INDEX nodes_ip_metadata_addresses ON nodes(address)
  WHERE address IS NOT NULL AND status != 'revoked';
