ALTER TABLE tunnel_intents ADD COLUMN control_port INTEGER CHECK(control_port BETWEEN 1024 AND 65535);
ALTER TABLE tunnel_intents ADD COLUMN path_key TEXT;
CREATE UNIQUE INDEX tunnel_control_claim ON tunnel_intents(listener_node_id, control_port);
