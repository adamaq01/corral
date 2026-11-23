alter table users add column credentials_changed_at timestamptz;

alter table sandboxes add column started_at timestamptz;
