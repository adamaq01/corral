alter table sandboxes add column delete_at text;

alter table users add column deletes_from_inside integer not null default 0;
