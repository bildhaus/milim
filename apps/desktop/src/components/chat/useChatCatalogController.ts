import { useEffect, useRef, useState } from "react";
import {
  listProviders,
  listSkills,
  listTools,
  loadStartupModels,
  type AccountRuntimeEnablement,
  type ModelInfo,
  type ProviderInfo,
  type SkillInfo,
  type ToolInfo,
} from "../../api";

export function useChatCatalogController(
  accountRuntimeEnabled: AccountRuntimeEnablement,
  skillsRevision: number,
) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelsLoaded, setModelsLoaded] = useState(false);
  const [modelsSettled, setModelsSettled] = useState(false);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [composerTools, setComposerTools] = useState<ToolInfo[]>([]);
  const modelsRef = useRef<ModelInfo[]>([]);

  useEffect(() => {
    modelsRef.current = models;
  }, [models]);

  useEffect(() => {
    let cancelled = false;
    setModelsSettled(false);
    void loadStartupModels(
      (nextModels) => {
        if (cancelled) return;
        modelsRef.current = nextModels;
        setModels(nextModels);
        setModelsLoaded(true);
      },
      accountRuntimeEnabled,
      modelsRef.current,
    ).finally(() => {
      if (!cancelled) setModelsSettled(true);
    });
    return () => {
      cancelled = true;
    };
  }, [accountRuntimeEnabled]);

  useEffect(() => {
    void listProviders().then(setProviders);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    const load = (attempt: number) => {
      void listSkills().then((next) => {
        if (cancelled) return;
        setSkills(next);
        if (next.length === 0 && attempt < 2) {
          retryTimer = setTimeout(() => load(attempt + 1), 250 * (attempt + 1));
        }
      });
    };
    load(0);
    return () => {
      cancelled = true;
      if (retryTimer) clearTimeout(retryTimer);
    };
  }, [skillsRevision]);

  useEffect(() => {
    let cancelled = false;
    void listTools()
      .then((tools) => {
        if (!cancelled) setComposerTools(tools);
      })
      .catch(() => {
        if (!cancelled) setComposerTools([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return {
    models,
    modelsLoaded,
    modelsSettled,
    providers,
    skills,
    composerTools,
    setModels,
    setProviders,
    setComposerTools,
  };
}
