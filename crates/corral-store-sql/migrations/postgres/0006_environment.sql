create table environment_entries (
    id uuid primary key,
    owner uuid not null references users on delete cascade,
    kind text not null,
    target text collate "C" not null,
    template boolean not null,
    apply_to_new boolean not null,
    content bytea,
    secret_id uuid,
    size bigint not null,
    glimpse_length bigint,
    glimpse_head text,
    glimpse_tail text,
    set_at timestamptz not null,
    created_at timestamptz not null,
    check ((content is null) <> (secret_id is null))
);

create index environment_entries_owner on environment_entries (owner);

create table environment_attachments (
    sandbox_id text not null references sandboxes on delete cascade,
    entry_id uuid not null references environment_entries on delete cascade,
    override_content bytea,
    override_secret_id uuid,
    override_size bigint,
    override_glimpse_length bigint,
    override_glimpse_head text,
    override_glimpse_tail text,
    override_set_at timestamptz,
    primary key (sandbox_id, entry_id),
    check (override_content is null or override_secret_id is null)
);

create index environment_attachments_entry on environment_attachments (entry_id);

alter table sandboxes add column environment_changed_at timestamptz;

update sandboxes set environment_changed_at = users.credentials_changed_at
    from users where users.id = sandboxes.owner;

alter table users drop column credentials_changed_at;

create table credentials_to_adopt (
    user_id uuid primary key references users on delete cascade
);

insert into credentials_to_adopt (user_id) select id from users;
