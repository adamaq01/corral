alter table sandboxes add column idle_timeout_secs bigint;

alter table sandboxes add column keep_awake_until timestamptz;

create table sandbox_client_states (
    sandbox_id text not null references sandboxes on delete cascade,
    client text not null,
    state text not null,
    primary key (sandbox_id, client)
);
