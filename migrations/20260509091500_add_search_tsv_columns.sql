ALTER TABLE threads
ADD COLUMN search_tsv tsvector
GENERATED ALWAYS AS (
    to_tsvector('english', COALESCE(title, ''))
) STORED;

CREATE INDEX threads_search_tsv_idx
    ON threads
    USING GIN (search_tsv);

ALTER TABLE posts
ADD COLUMN search_tsv tsvector
GENERATED ALWAYS AS (
    to_tsvector('english', COALESCE(body, ''))
) STORED;

CREATE INDEX posts_search_tsv_idx
    ON posts
    USING GIN (search_tsv);
