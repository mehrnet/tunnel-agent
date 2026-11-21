ALTER TABLE tunnel_intents ADD COLUMN forward_udp INTEGER NOT NULL DEFAULT 0 CHECK(forward_udp IN (0,1));
