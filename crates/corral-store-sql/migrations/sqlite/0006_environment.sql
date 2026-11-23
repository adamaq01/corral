create table environment_entries (
    id blob primary key not null,
    owner blob not null references users on delete cascade,
    kind text not null,
    target text not null,
    template integer not null,
    apply_to_new integer not null,
    content blob,
    secret_id blob,
    size integer not null,
    glimpse_length integer,
    glimpse_head text,
    glimpse_tail text,
    set_at text not null,
    created_at text not null,
    check ((content is null) <> (secret_id is null))
) strict;

create index environment_entries_owner on environment_entries (owner);

create table environment_attachments (
    sandbox_id text not null references sandboxes on delete cascade,
    entry_id blob not null references environment_entries on delete cascade,
    override_content blob,
    override_secret_id blob,
    override_size integer,
    override_glimpse_length integer,
    override_glimpse_head text,
    override_glimpse_tail text,
    override_set_at text,
    primary key (sandbox_id, entry_id),
    check (override_content is null or override_secret_id is null)
) strict;

create index environment_attachments_entry on environment_attachments (entry_id);

alter table sandboxes add column environment_changed_at text;

update sandboxes set environment_changed_at =
    (select credentials_changed_at from users where users.id = sandboxes.owner);

alter table users drop column credentials_changed_at;

create table credentials_to_adopt (
    user_id blob primary key not null references users on delete cascade
) strict;

insert into credentials_to_adopt (user_id) select id from users;
