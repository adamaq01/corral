create table settings (
    key text primary key not null,
    value text not null,
    updated_at text not null
) strict;

create table member_allowances (
    user_id blob primary key not null references users on delete cascade,
    max_sandboxes integer not null
) strict;
