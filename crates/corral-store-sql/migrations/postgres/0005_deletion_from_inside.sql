alter table sandboxes add column delete_at timestamptz;

alter table users add column deletes_from_inside boolean not null default false;
