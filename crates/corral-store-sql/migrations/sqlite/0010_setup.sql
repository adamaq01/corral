alter table environment_entries rename to setup_entries;

alter table environment_attachments rename to setup_attachments;

alter table sandboxes rename column environment_changed_at to setup_changed_at;

drop index environment_entries_owner;

drop index environment_attachments_entry;

create table setup_entries_new (
    id blob primary key not null,
    owner blob references users on delete cascade,
    shared text check (shared in ('required', 'offered')),
    kind text not null,
    target text not null,
    template integer not null,
    executable integer not null default 0,
    apply_to_new integer not null,
    content blob,
    secret_id blob,
    size integer not null,
    glimpse_length integer,
    glimpse_head text,
    glimpse_tail text,
    set_at text not null,
    created_at text not null,
    check ((content is null) <> (secret_id is null)),
    check ((owner is null) <> (shared is null))
) strict;

insert into setup_entries_new (id, owner, kind, target, template, executable, apply_to_new,
    content, secret_id, size, glimpse_length, glimpse_head, glimpse_tail, set_at, created_at)
select id, owner, kind, target, template, executable, apply_to_new, content, secret_id, size,
    glimpse_length, glimpse_head, glimpse_tail, set_at, created_at
from setup_entries;

-- Dropping a table with foreign keys on deletes its rows first, which would cascade to the
-- attachments, so they move to a table that refers to the new entries before the old ones go.
create table setup_attachments_new (
    sandbox_id text not null references sandboxes on delete cascade,
    entry_id blob not null references setup_entries_new on delete cascade,
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

insert into setup_attachments_new select * from setup_attachments;

drop table setup_attachments;

drop table setup_entries;

alter table setup_entries_new rename to setup_entries;

alter table setup_attachments_new rename to setup_attachments;

create index setup_entries_owner on setup_entries (owner);

create index setup_entries_shared on setup_entries (shared) where shared is not null;

create index setup_attachments_entry on setup_attachments (entry_id);
