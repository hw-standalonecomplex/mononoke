CREATE TABLE changesets (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  repo_id INTEGER NOT NULL,
  cs_id BINARY(20) NOT NULL,
  gen BIGINT NOT NULL,
  UNIQUE (repo_id, cs_id)
);

CREATE TABLE csparents (
  cs_id BIGINT NOT NULL,
  parent_id BIGINT NOT NULL,
  seq INTEGER NOT NULL,
  PRIMARY KEY (cs_id, parent_id, seq)
);
