-- Bearer-token-authenticated reverse proxy for API tooling (a coding
-- assistant, a script) pointed at Ollama/vLLM/SGLang — distinct from the
-- session-cookie-authenticated /proxy/ route above, which requires a
-- browser and a Helve login. See backend/src/models_proxy.rs and the
-- README's "/models/" section.
--
-- engine_slug disambiguates concurrent engines for one user
-- (/models/<username>/<engine_slug>/...); api_proxy_enabled opts a
-- template into generating a proxy_token at launch time the same way
-- secret_env_key already opts one into generate_secret_for.
ALTER TABLE templates ADD COLUMN engine_slug TEXT;
ALTER TABLE templates ADD COLUMN api_proxy_enabled BOOLEAN NOT NULL DEFAULT false;

-- proxy_token is the actual authority: a request's bearer token is looked
-- up here directly (hash-free, same plaintext-equality convention
-- proxy_auth_tokens/proxy_sessions already use in this table), and the
-- path's {username}/{engine_slug}/{model_slug} segments are only a
-- consistency check against what the matched row says. model_slug is
-- NULL whenever the deployment's own `model` field is unset.
ALTER TABLE deployment_secrets ADD COLUMN proxy_token TEXT;
ALTER TABLE deployment_secrets ADD COLUMN engine_slug TEXT;
ALTER TABLE deployment_secrets ADD COLUMN model_slug TEXT;
CREATE UNIQUE INDEX deployment_secrets_proxy_token_idx
    ON deployment_secrets (proxy_token) WHERE proxy_token IS NOT NULL;

-- Only changes the templates table, so an already-launched deployment
-- doesn't retroactively gain a proxy token or /models/ URL — same
-- precedent as every other template-default change here. Users get one on
-- their next launch/relaunch.
UPDATE templates SET engine_slug = 'ollama', api_proxy_enabled = true WHERE name = 'Ollama';
UPDATE templates SET engine_slug = 'vllm',   api_proxy_enabled = true WHERE name = 'vLLM';
UPDATE templates SET engine_slug = 'sglang', api_proxy_enabled = true WHERE name = 'SGLang';
