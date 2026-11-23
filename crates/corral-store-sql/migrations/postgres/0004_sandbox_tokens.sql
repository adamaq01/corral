alter table sandboxes add column token_hash text;

create unique index sandboxes_token_hash on sandboxes (token_hash);
