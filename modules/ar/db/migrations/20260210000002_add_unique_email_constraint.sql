-- Add unique constraint on (app_id, email) to prevent duplicate emails per app
ALTER TABLE ar_customers ADD CONSTRAINT unique_app_email UNIQUE (app_id, email);
