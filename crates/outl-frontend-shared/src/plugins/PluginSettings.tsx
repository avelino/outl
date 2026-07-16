/**
 * `<PluginSettings />` — the one plugin config form, shared by desktop + mobile.
 *
 * Renders a plugin's config schema as an editable form: plaintext config fields
 * (string / number / boolean) write to the lockfile via `plugin_config_set`
 * (the host coerces + reloads), and secret fields write to the OS keychain via
 * `plugin_secret_set` — their value never crosses the wire, the form only shows
 * set / not-set.
 *
 * Unlike the dumb `<PeerList />`, this component owns its own fetch + mutate
 * lifecycle (both clients want identical behavior), so a client just drops
 * `<PluginSettings pluginId={id} />`. Markup is neutral class names
 * (`outl-plugin-settings__*`); import `@outl/shared/plugins/styles` for a
 * theme-agnostic baseline each client can override.
 */

import { For, Show, createSignal, onMount, type JSX } from "solid-js";

import {
  pluginConfigSet,
  pluginSecretRemove,
  pluginSecretSet,
  pluginSettingsDescribe,
} from "../api/commands";
import type { PluginSettingsField } from "../api/types";

interface PluginSettingsProps {
  /** Reverse-DNS id of the installed plugin to configure. */
  pluginId: string;
  /** Rendered when the plugin exposes no config schema. */
  emptyState?: JSX.Element;
}

/** Coerce a field's current value (or default) to a text-input string. */
function inputValue(field: PluginSettingsField): string {
  const v = field.value ?? field.default;
  return v == null ? "" : String(v);
}

export function PluginSettings(props: PluginSettingsProps): JSX.Element {
  const [fields, setFields] = createSignal<PluginSettingsField[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [busyKey, setBusyKey] = createSignal<string | null>(null);
  const [secretDraft, setSecretDraft] = createSignal<Record<string, string>>({});

  async function refresh() {
    try {
      setFields(await pluginSettingsDescribe(props.pluginId));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }
  onMount(refresh);

  async function withBusy(key: string, fn: () => Promise<unknown>) {
    setBusyKey(key);
    try {
      await fn();
      setError(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyKey(null);
    }
  }

  const setConfig = (key: string, value: string) =>
    withBusy(key, () => pluginConfigSet(props.pluginId, key, value));

  const saveSecret = (key: string) => {
    const value = secretDraft()[key] ?? "";
    if (!value) return;
    return withBusy(key, async () => {
      await pluginSecretSet(props.pluginId, key, value);
      setSecretDraft({ ...secretDraft(), [key]: "" });
    });
  };

  const clearSecret = (key: string) =>
    withBusy(key, () => pluginSecretRemove(props.pluginId, key));

  return (
    <div class="outl-plugin-settings">
      <Show when={error()}>
        <div class="outl-plugin-settings__error" role="alert">
          {error()}
        </div>
      </Show>

      <Show when={!loading()} fallback={<div class="outl-plugin-settings__loading">Loading…</div>}>
        <Show
          when={fields().length > 0}
          fallback={
            props.emptyState ?? (
              <div class="outl-plugin-settings__empty">This plugin has no configurable fields.</div>
            )
          }
        >
          <For each={fields()}>
            {(field) => {
              const busy = () => busyKey() === field.key;
              return (
                <div class="outl-plugin-settings__field" data-secret={field.secret}>
                  <label class="outl-plugin-settings__label" for={`ops-${field.key}`}>
                    {field.title}
                    <Show when={field.secret}>
                      <span
                        class="outl-plugin-settings__badge"
                        data-set={field.isSet}
                      >
                        {field.isSet ? "set" : "not set"}
                      </span>
                    </Show>
                  </label>
                  <Show when={field.description}>
                    <p class="outl-plugin-settings__hint">{field.description}</p>
                  </Show>

                  <Show
                    when={field.secret}
                    fallback={
                      <Show
                        when={field.kind === "boolean"}
                        fallback={
                          <input
                            id={`ops-${field.key}`}
                            class="outl-plugin-settings__input"
                            type={
                              field.kind === "integer" || field.kind === "number"
                                ? "number"
                                : "text"
                            }
                            value={inputValue(field)}
                            disabled={busy()}
                            onChange={(e) => setConfig(field.key, e.currentTarget.value)}
                          />
                        }
                      >
                        <input
                          id={`ops-${field.key}`}
                          class="outl-plugin-settings__checkbox"
                          type="checkbox"
                          checked={field.value === true}
                          disabled={busy()}
                          onChange={(e) =>
                            setConfig(field.key, e.currentTarget.checked ? "true" : "false")
                          }
                        />
                      </Show>
                    }
                  >
                    <div class="outl-plugin-settings__secret">
                      <input
                        id={`ops-${field.key}`}
                        class="outl-plugin-settings__input"
                        type="password"
                        autocomplete="off"
                        placeholder={field.isSet ? "••••••• (replace)" : "Enter value"}
                        value={secretDraft()[field.key] ?? ""}
                        disabled={busy()}
                        onInput={(e) =>
                          setSecretDraft({ ...secretDraft(), [field.key]: e.currentTarget.value })
                        }
                        onKeyDown={(e) => {
                          if (e.key === "Enter") saveSecret(field.key);
                        }}
                      />
                      <button
                        type="button"
                        class="outl-plugin-settings__btn"
                        disabled={busy() || !(secretDraft()[field.key] ?? "")}
                        onClick={() => saveSecret(field.key)}
                      >
                        Save
                      </button>
                      <Show when={field.isSet}>
                        <button
                          type="button"
                          class="outl-plugin-settings__btn outl-plugin-settings__btn--danger"
                          disabled={busy()}
                          onClick={() => clearSecret(field.key)}
                        >
                          Clear
                        </button>
                      </Show>
                    </div>
                  </Show>
                </div>
              );
            }}
          </For>
        </Show>
      </Show>
    </div>
  );
}
