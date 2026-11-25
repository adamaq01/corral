alter table sandboxes add column idle_timeout_secs integer;

alter table sandboxes add column keep_awake_until text;

create table sandbox_client_states (
    sandbox_id text not null references sandboxes on delete cascade,
    client text not null,
    state text not null,
    primary key (sandbox_id, client)
) strict;
