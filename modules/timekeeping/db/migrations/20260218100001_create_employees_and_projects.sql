-- Timekeeping: Employees, Projects, and Tasks
--
-- Employee directory for time tracking. External payroll ID supports
-- integration with third-party payroll systems. Project and task catalog
-- provides the structure for time allocation and cost tracking.
--
-- All tables tenant-scoped via app_id.

-- ============================================================
-- EMPLOYEES
-- ============================================================

CREATE TABLE tk_employees (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id              VARCHAR(50) NOT NULL,
    employee_code       VARCHAR(50) NOT NULL,
    first_name          VARCHAR(255) NOT NULL,
    last_name           VARCHAR(255) NOT NULL,
    email               VARCHAR(255),
    department          VARCHAR(255),
    -- Mapping to external payroll systems (e.g. ADP, Gusto employee ID)
    external_payroll_id VARCHAR(255),
    hourly_rate_minor   BIGINT,            -- optional default rate in minor units (cents)
    currency            VARCHAR(3) NOT NULL DEFAULT 'USD',
    active              BOOLEAN NOT NULL DEFAULT TRUE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT tk_employees_app_code_unique
        UNIQUE (app_id, employee_code)
);

CREATE INDEX tk_employees_app_id
    ON tk_employees(app_id);
CREATE INDEX tk_employees_app_active
    ON tk_employees(app_id, active);
CREATE INDEX tk_employees_external_payroll
    ON tk_employees(app_id, external_payroll_id)
    WHERE external_payroll_id IS NOT NULL;

-- ============================================================
-- PROJECTS
-- ============================================================

CREATE TABLE tk_projects (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id           VARCHAR(50) NOT NULL,
    project_code     VARCHAR(50) NOT NULL,
    name             VARCHAR(255) NOT NULL,
    description      TEXT,
    billable         BOOLEAN NOT NULL DEFAULT FALSE,
    -- GL account ref for cost allocation (soft reference to gl.accounts)
    gl_account_ref   VARCHAR(100),
    active           BOOLEAN NOT NULL DEFAULT TRUE,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT tk_projects_app_code_unique
        UNIQUE (app_id, project_code)
);

CREATE INDEX tk_projects_app_id
    ON tk_projects(app_id);
CREATE INDEX tk_projects_app_active
    ON tk_projects(app_id, active);

-- ============================================================
-- TASKS (belong to a project)
-- ============================================================

CREATE TABLE tk_tasks (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id       VARCHAR(50) NOT NULL,
    project_id   UUID NOT NULL REFERENCES tk_projects(id) ON DELETE RESTRICT,
    task_code    VARCHAR(50) NOT NULL,
    name         VARCHAR(255) NOT NULL,
    active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT tk_tasks_project_code_unique
        UNIQUE (project_id, task_code)
);

CREATE INDEX tk_tasks_app_id
    ON tk_tasks(app_id);
CREATE INDEX tk_tasks_project_id
    ON tk_tasks(project_id);
