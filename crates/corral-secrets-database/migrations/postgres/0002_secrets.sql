create table secrets (
    user_id uuid not null,
    id uuid not null,
    sealed bytea not null,
    primary key (user_id, id)
);
