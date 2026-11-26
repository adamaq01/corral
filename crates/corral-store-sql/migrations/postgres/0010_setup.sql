alter table environment_entries rename to setup_entries;

alter table environment_attachments rename to setup_attachments;

alter table sandboxes rename column environment_changed_at to setup_changed_at;

alter index environment_entries_pkey rename to setup_entries_pkey;

alter index environment_entries_owner rename to setup_entries_owner;

alter index environment_attachments_pkey rename to setup_attachments_pkey;

alter index environment_attachments_entry rename to setup_attachments_entry;

alter table setup_entries alter column owner drop not null;

alter table setup_entries add column shared text null check (shared in ('required', 'offered'));

alter table setup_entries add check ((owner is null) <> (shared is null));

create index setup_entries_shared on setup_entries (shared) where shared is not null;
