alter table environment_entries rename to setup_entries;

alter table environment_attachments rename to setup_attachments;

alter table sandboxes rename column environment_changed_at to setup_changed_at;

drop index environment_entries_owner;

create index setup_entries_owner on setup_entries (owner);

drop index environment_attachments_entry;

create index setup_attachments_entry on setup_attachments (entry_id);
