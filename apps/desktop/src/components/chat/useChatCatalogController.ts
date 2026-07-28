import { useEffect, useState } from "react";
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
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [composerTools, setComposerTools] = useState<ToolInfo[]>([]);

  useEffect(() => {
    let cancelled = false;
    void loadStartupModels(
      (nextModels) => {
        if (cancelled) return;
        setModels(nextModels);
        setModelsLoaded(true);
      },
      accountRuntimeEnabled,
    );
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
    providers,
    skills,
    composerTools,
    setModels,
    setProviders,
    setComposerTools,
  };
}
