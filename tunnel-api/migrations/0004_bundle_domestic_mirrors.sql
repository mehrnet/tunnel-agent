ALTER TABLE bundle_releases ADD COLUMN domestic_urls TEXT;

UPDATE bundle_releases
SET domestic_urls = CASE
  WHEN domestic_url IS NULL THEN '[]'
  ELSE '["' || domestic_url || '"]'
END
WHERE domestic_urls IS NULL;
