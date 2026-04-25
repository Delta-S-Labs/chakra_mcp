-- First-login survey responses.
--
-- Network operators opt in via SURVEY_ENABLED=true on the app service.
-- The hosted public network always opts in (research signal). Per-user,
-- the survey is shown exactly once — completed_at != NULL means done.
--
-- We deliberately don't store free-form text under a UNIQUE constraint
-- so the user can resubmit/update in a future flow if we ever expose it.

CREATE TABLE IF NOT EXISTS surveys (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE UNIQUE,

    -- "What kind of work" — short multiple-choice.
    use_case TEXT,             -- e.g. "personal-tools", "internal-tooling",
                               --      "customer-product", "research"

    -- Categories of agents the user builds. Multi-select.
    agent_types TEXT[] NOT NULL DEFAULT '{}',
    -- e.g. {coding, research, calendar, support, voice, browser, robotics}

    -- Frameworks the user reaches for. Multi-select.
    frameworks TEXT[] NOT NULL DEFAULT '{}',
    -- e.g. {langchain, llamaindex, mastra, vercel-ai-sdk, rig,
    --       crewai, autogen, custom}

    -- Stage / scale.
    scale TEXT,                -- "exploring" | "team" | "company" | "production"

    -- Free-form follow-up.
    notes TEXT,

    completed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_surveys_user ON surveys (user_id);
