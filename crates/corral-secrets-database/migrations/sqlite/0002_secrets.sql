create table secrets (
    user_id blob not null,
    id blob not null,
    sealed blob not null,
    primary key (user_id, id)
) strict;
