CREATE TABLE owner_testing_clusters (
  owner_id TEXT PRIMARY KEY REFERENCES owners(id) ON DELETE CASCADE,
  cluster_id TEXT NOT NULL UNIQUE REFERENCES clusters(id) ON DELETE CASCADE
) STRICT;

ALTER TABLE bundle_releases ADD COLUMN testing INTEGER NOT NULL DEFAULT 0 CHECK(testing IN (0,1));
CREATE UNIQUE INDEX one_testing_bundle_per_architecture ON bundle_releases(architecture) WHERE testing=1;

INSERT INTO owner_testing_clusters(owner_id,cluster_id)
SELECT c.owner_id,MIN(c.id)
FROM seed_canary_releases r JOIN cluster_nodes m ON m.node_id=r.node_id JOIN clusters c ON c.id=m.cluster_id
GROUP BY c.owner_id HAVING COUNT(DISTINCT c.id)=1;
