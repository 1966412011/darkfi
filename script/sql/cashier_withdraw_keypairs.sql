CREATE TABLE IF NOT EXISTS withdraw_keypairs(
	keypair_id INTEGER PRIMARY KEY NOT NULL,
	token_key_public BLOB NOT NULL,
	d_key_secret BLOB NOT NULL,
	d_key_public BLOB NOT NULL,
	network BLOB NOT NULL,
	token_id BLOB NOT NULL,
	mint_address BLOB NOT NULL,
	confirm BLOB NOT NULL
);