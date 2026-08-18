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
    void listSkills().then(setSkills);
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
