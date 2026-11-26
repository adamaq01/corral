alter table environment_entries rename to setup_entries;

alter table environment_attachments rename to setup_attachments;

alter table sandboxes rename column environment_changed_at to setup_changed_at;

alter index environment_entries_pkey rename to setup_entries_pkey;

alter index environment_entries_owner rename to setup_entries_owner;

alter index environment_attachments_pkey rename to setup_attachments_pkey;

alter index environment_attachments_entry rename to setup_attachments_entry;
