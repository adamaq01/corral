create table credentials (
    user_id blob not null,
    name text not null,
    sealed blob not null,
    primary key (user_id, name)
) strict;
