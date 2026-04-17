-- Training delivery extension for Workforce Competence
--
-- Three tables:
--   wc_training_plans         — plan registry (what training is offered)
--   wc_training_assignments   — operator→plan assignments with status tracking
--   wc_training_completions   — completion records; outcome=passed auto-creates competence assignment

CREATE TABLE wc_training_plans (
    id                          UUID PRIMARY KEY,
    tenant_id                   TEXT NOT NULL,
    plan_code                   TEXT NOT NULL,
    title                       TEXT NOT NULL,
    description                 TEXT,
    artifact_id                 UUID NOT NULL REFERENCES wc_competence_artifacts(id),
    duration_minutes            INT NOT NULL CHECK (duration_minutes > 0),
    instructor_id               UUID,
    material_refs               TEXT[] NOT NULL DEFAULT '{}',
    required_for_artifact_codes TEXT[] NOT NULL DEFAULT '{}',
    location                    TEXT,
    scheduled_at                TIMESTAMP WITH TIME ZONE,
    active                      BOOLEAN NOT NULL DEFAULT true,
    created_at                  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_by                  TEXT,

    CONSTRAINT wc_training_plans_tenant_code_unique UNIQUE (tenant_id, plan_code)
);

CREATE INDEX idx_wc_tp_tenant   ON wc_training_plans(tenant_id);
CREATE INDEX idx_wc_tp_artifact ON wc_training_plans(tenant_id, artifact_id);

CREATE TABLE wc_training_assignments (
    id           UUID PRIMARY KEY,
    tenant_id    TEXT NOT NULL,
    plan_id      UUID NOT NULL REFERENCES wc_training_plans(id),
    operator_id  UUID NOT NULL,
    assigned_by  TEXT NOT NULL,
    assigned_at  TIMESTAMP WITH TIME ZONE NOT NULL,
    status       TEXT NOT NULL DEFAULT 'assigned'
                 CHECK (status IN ('assigned','scheduled','in_progress','completed','cancelled','no_show')),
    scheduled_at TIMESTAMP WITH TIME ZONE,
    notes        TEXT,
    updated_at   TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_wc_ta_tenant_plan     ON wc_training_assignments(tenant_id, plan_id);
CREATE INDEX idx_wc_ta_tenant_operator ON wc_training_assignments(tenant_id, operator_id);

CREATE TABLE wc_training_completions (
    id                                UUID PRIMARY KEY,
    tenant_id                         TEXT NOT NULL,
    assignment_id                     UUID NOT NULL REFERENCES wc_training_assignments(id),
    operator_id                       UUID NOT NULL,
    plan_id                           UUID NOT NULL REFERENCES wc_training_plans(id),
    completed_at                      TIMESTAMP WITH TIME ZONE NOT NULL,
    verified_by                       TEXT,
    outcome                           TEXT NOT NULL
                                      CHECK (outcome IN ('passed','failed','incomplete')),
    notes                             TEXT,
    resulting_competence_assignment_id UUID REFERENCES wc_operator_competences(id),
    created_at                        TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT wc_training_completions_assignment_unique UNIQUE (assignment_id)
);

CREATE INDEX idx_wc_tc_tenant_plan     ON wc_training_completions(tenant_id, plan_id);
CREATE INDEX idx_wc_tc_tenant_operator ON wc_training_completions(tenant_id, operator_id);
CREATE INDEX idx_wc_tc_assignment      ON wc_training_completions(assignment_id);
