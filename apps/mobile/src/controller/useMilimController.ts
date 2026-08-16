import {useCallback, useEffect, useRef, useState} from 'react';
import {AppState, type AppStateStatus} from 'react-native';
import {
  cleanupAppearanceBackgrounds,
  fetchAppearanceBackground,
} from '../appearance';
import {
  claimPairing,
  connectControlSocket,
  fetchBootstrap,
  fetchMobileHostProbe,
  fetchTimeline,
  newCommandId,
  normalizeEndpoint,
  sendCommand,
} from '../control/client';
import {
  applyControlEvent,
  applyTimelinePage,
  emptyReplica,
  type TimelineReplica,
} from '../control/replica';
import type {
  ControlBootstrapV1,
  ControlCommandResultV1,
  ControlCommandV1,
  JsonValue,
  SavedHost,
} from '../control/types';
import {isProtocolCompatible} from '../control/types';
import {discoverMilimHosts} from '../discovery';
import {assertHostIdentity, parsePairingClaim} from '../pairing';
import {
  listHosts,
  readDraft,
  readTimelineTail,
  removeHost as removeCachedHost,
  saveDraft as persistDraft,
  saveHost,
  saveTimelineTail,
} from '../storage/cache';
import {
  readDeviceCredential,
  removeDeviceCredential,
  saveDeviceCredential,
} from '../storage/secure';

export type ConnectionStatus = 'offline' | 'connecting' | 'online' | 'incompatible';

function pageFromReplica(replica: TimelineReplica) {
  const items = replica.items.slice(-150);
  return {
    thread_id: replica.threadId,
    epoch: replica.epoch,
    first_seq: items.at(0)?.seq ?? null,
    last_seq: items.at(-1)?.seq ?? null,
    has_older: replica.hasOlder || replica.items.length > items.length,
    has_newer: replica.hasNewer,
    before_seq: null,
    after_seq: null,
    items,
  };
}

export function useMilimController() {
  const [hosts, setHosts] = useState<SavedHost[]>([]);
  const [activeHost, setActiveHostState] = useState<SavedHost | null>(null);
  const [bootstrap, setBootstrap] = useState<ControlBootstrapV1 | null>(null);
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(null);
  const [timeline, setTimeline] = useState<TimelineReplica | null>(null);
  const [draft, setDraftState] = useState('');
  const [status, setStatus] = useState<ConnectionStatus>('offline');
  const [lastError, setLastError] = useState<string | null>(null);
  const [pendingRetry, setPendingRetry] = useState<ControlCommandV1 | null>(null);
  const [appearanceBackgroundUri, setAppearanceBackgroundUri] = useState<string | null>(null);
  const [appState, setAppState] = useState<AppStateStatus>(
    () => (AppState.currentState || 'active') as AppStateStatus,
  );
  const socket = useRef<WebSocket | null>(null);
  const credential = useRef<string | null>(null);
  const activeHostRef = useRef<SavedHost | null>(null);
  const selectedThreadRef = useRef<string | null>(null);
  const timelineRef = useRef<TimelineReplica | null>(null);
  const bootstrapRefreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    listHosts()
      .then(saved => {
        setHosts(saved);
        setActiveHostState(saved[0] ?? null);
      })
      .catch(error => setLastError(String(error)));
    const subscription = AppState.addEventListener('change', setAppState);
    return () => subscription.remove();
  }, []);

  useEffect(() => {
    timelineRef.current = timeline;
  }, [timeline]);
  activeHostRef.current = activeHost;
  selectedThreadRef.current = selectedThreadId;

  const rememberHost = useCallback(async (host: SavedHost) => {
    await saveHost(host);
    setHosts(current => [host, ...current.filter(item => item.hostId !== host.hostId)]);
    setActiveHostState(host);
  }, []);

  const tryBootstrap = useCallback(async (host: SavedHost) => {
    const key = await readDeviceCredential(host.hostId);
    if (!key) throw new Error('This host no longer has a device credential. Pair it again.');
    credential.current = key;
    const savedCandidates = [host.lastSuccessfulUrl, ...host.candidates].filter(
      (value, index, all): value is string => Boolean(value) && all.indexOf(value) === index,
    );
    let lastFailure: unknown = null;
    for (const endpoint of savedCandidates) {
      try {
        const result = await fetchBootstrap(endpoint, key);
        assertHostIdentity(host.hostId, result.host_id);
        return {endpoint, result};
      } catch (error) {
        lastFailure = error;
      }
    }
    const discovered = await discoverMilimHosts().catch(() => []);
    const candidates = discovered
      .filter(item => !item.hostId || item.hostId === host.hostId)
      .map(item => item.endpoint);
    for (const endpoint of candidates) {
      try {
        const result = await fetchBootstrap(endpoint, key);
        assertHostIdentity(host.hostId, result.host_id);
        return {endpoint, result};
      } catch (error) {
        lastFailure = error;
      }
    }
    throw lastFailure ?? new Error('milim desktop is unavailable.');
  }, []);

  const refreshBootstrap = useCallback(async () => {
    const host = activeHostRef.current;
    if (!host || !credential.current || !host.lastSuccessfulUrl) return null;
    const next = await fetchBootstrap(host.lastSuccessfulUrl, credential.current);
    assertHostIdentity(host.hostId, next.host_id);
    setBootstrap(current =>
      current?.appearance?.revision &&
      current.appearance.revision === next.appearance?.revision
        ? {...next, appearance: current.appearance}
        : next,
    );
    return next;
  }, []);

  const refreshTimeline = useCallback(
    async (mode: 'tail' | 'after' | 'before' = 'after') => {
      const host = activeHostRef.current;
      const threadId = selectedThreadRef.current;
      if (!host?.lastSuccessfulUrl || !credential.current || !threadId) return;
      const current = timelineRef.current ?? emptyReplica(threadId);
      if (mode === 'before' && current.firstSeq === null) return;
      const query = mode === 'before'
        ? {beforeSeq: current.firstSeq!}
        : mode === 'after' && current.lastSeq !== null
          ? {afterSeq: current.lastSeq}
          : {tail: 150};
      const page = await fetchTimeline(
        host.lastSuccessfulUrl,
        credential.current,
        threadId,
        query,
      );
      let next = applyTimelinePage(current, page, mode);
      if (next.needsTailRefresh) {
        const tail = await fetchTimeline(
          host.lastSuccessfulUrl,
          credential.current,
          threadId,
          {tail: 150},
        );
        next = applyTimelinePage(next, tail, 'tail');
      }
      setTimeline(next);
      timelineRef.current = next;
      await saveTimelineTail(host.hostId, pageFromReplica(next));
    },
    [],
  );

  useEffect(() => {
    const host = activeHostRef.current;
    socket.current?.close();
    socket.current = null;
    setBootstrap(null);
    setStatus(host ? 'connecting' : 'offline');
    if (!host || appState !== 'active') return;
    let cancelled = false;
    void tryBootstrap(host)
      .then(async ({endpoint, result}) => {
        if (cancelled) return;
        if (!isProtocolCompatible(result.protocol)) {
          setStatus('incompatible');
          setBootstrap(result);
          return;
        }
        const connected: SavedHost = {
          ...host,
          displayName: result.host_name,
          protocol: result.protocol,
          lastSuccessfulUrl: endpoint,
          candidates: [endpoint, ...host.candidates.filter(value => value !== endpoint)],
          lastConnectedAt: Date.now(),
        };
        await rememberHost(connected);
        if (cancelled) return;
        setBootstrap(result);
        setSelectedThreadId(current =>
          current && result.threads.some(thread => thread.id === current)
            ? current
            : (result.threads.find(thread => !thread.archived_at_ms)?.id ?? null),
        );
        setStatus('online');
        setLastError(null);
        socket.current = await connectControlSocket(
          endpoint,
          credential.current!,
          event => {
            if (event.host_id !== host.hostId) {
              setLastError('Ignored an event from a different milim desktop.');
              return;
            }
            const currentTimeline = timelineRef.current;
            if (currentTimeline) {
              const next = applyControlEvent(currentTimeline, event, host.hostId);
              timelineRef.current = next;
              setTimeline(next);
            }
            if (
              event.type.startsWith('thread.') ||
              event.type.startsWith('run.') ||
              event.type.startsWith('turn.') ||
              event.type.startsWith('approval_') ||
              event.type.startsWith('worker.') ||
              event.type === 'appearance.updated' ||
              event.type === 'timeline.appended' ||
              event.type === 'sync.required'
            ) {
              if (bootstrapRefreshTimer.current) clearTimeout(bootstrapRefreshTimer.current);
              bootstrapRefreshTimer.current = setTimeout(() => void refreshBootstrap(), 120);
            }
          },
          () => {
            if (!cancelled && AppState.currentState === 'active') setStatus('offline');
          },
        );
      })
      .catch(error => {
        if (!cancelled) {
          setStatus('offline');
          setLastError(error instanceof Error ? error.message : String(error));
        }
      });
    return () => {
      cancelled = true;
      socket.current?.close();
      socket.current = null;
    };
  }, [activeHost?.hostId, appState, rememberHost, refreshBootstrap, tryBootstrap]);

  useEffect(() => {
    const host = activeHostRef.current;
    const appearance = bootstrap?.appearance;
    if (!host || bootstrap?.host_id !== host.hostId || !appearance?.background.has_image) {
      setAppearanceBackgroundUri(null);
      if (host && bootstrap?.host_id === host.hostId) {
        void cleanupAppearanceBackgrounds(host.hostId);
      }
      return;
    }
    const endpoint = host.lastSuccessfulUrl;
    const key = credential.current;
    if (!endpoint || !key || bootstrap.capabilities.appearance_assets === false) return;
    let cancelled = false;
    setAppearanceBackgroundUri(current =>
      current?.includes(appearance.revision) ? current : null,
    );
    void fetchAppearanceBackground(
      endpoint,
      key,
      host.hostId,
      appearance.revision,
    )
      .then(uri => {
        if (!cancelled) setAppearanceBackgroundUri(uri);
        return cleanupAppearanceBackgrounds(host.hostId, appearance.revision);
      })
      .catch(error => console.warn('Could not mirror desktop background:', error));
    return () => {
      cancelled = true;
    };
  }, [
    activeHost?.hostId,
    bootstrap?.appearance,
    bootstrap?.appearance?.background.has_image,
    bootstrap?.appearance?.revision,
    bootstrap?.capabilities.appearance_assets,
    bootstrap?.host_id,
  ]);

  useEffect(() => {
    const host = activeHostRef.current;
    if (!host || !selectedThreadId) {
      setTimeline(null);
      setDraftState('');
      return;
    }
    let cancelled = false;
    Promise.all([
      readTimelineTail(host.hostId, selectedThreadId),
      readDraft(host.hostId, selectedThreadId),
    ]).then(([page, savedDraft]) => {
      if (cancelled) return;
      const cached = page
        ? applyTimelinePage(emptyReplica(selectedThreadId), page, 'tail')
        : emptyReplica(selectedThreadId);
      timelineRef.current = cached;
      setTimeline(cached);
      setDraftState(savedDraft);
      if (status === 'online') void refreshTimeline('after');
    });
    return () => {
      cancelled = true;
    };
  }, [activeHost?.hostId, refreshTimeline, selectedThreadId, status]);

  const setActiveHost = useCallback(
    (hostId: string) => {
      const host = hosts.find(item => item.hostId === hostId) ?? null;
      setSelectedThreadId(null);
      setActiveHostState(host);
    },
    [hosts],
  );

  const pair = useCallback(
    async (rawClaim: string, deviceName: string) => {
      const claim = parsePairingClaim(rawClaim);
      const supplied = normalizeEndpoint(claim.endpoint);
      const discovered = claim.hostId
        ? await discoverMilimHosts(2_500).catch(() => [])
        : [];
      const candidates = [
        supplied,
        ...discovered
          .filter(host => host.hostId === claim.hostId)
          .map(host => normalizeEndpoint(host.endpoint)),
      ].filter((value, index, values) => values.indexOf(value) === index);
      let endpoint: string | null = null;
      let lastFailure: unknown = null;
      for (const candidate of candidates) {
        try {
          const probe = await Promise.race([
            fetchMobileHostProbe(candidate),
            new Promise<null>(resolve => setTimeout(() => resolve(null), 2_000)),
          ]);
          if (!probe || probe.service !== 'milim-mobile-control') continue;
          assertHostIdentity(claim.hostId, probe.host_id);
          endpoint = candidate;
          break;
        } catch (error) {
          lastFailure = error;
        }
      }
      if (!endpoint) {
        throw lastFailure ?? new Error(
          'The paired desktop was not reachable at its saved URL or matching LAN discovery record.',
        );
      }
      const paired = await claimPairing(endpoint, claim.pairId, claim.secret, deviceName);
      const result = await fetchBootstrap(endpoint, paired.device_key);
      assertHostIdentity(claim.hostId, result.host_id);
      if (!isProtocolCompatible(result.protocol)) {
        throw new Error(
          `Desktop protocol ${result.protocol.min}-${result.protocol.max} is not compatible with mobile v1.`,
        );
      }
      await saveDeviceCredential(result.host_id, paired.device_key);
      credential.current = paired.device_key;
      const host: SavedHost = {
        hostId: result.host_id,
        displayName: result.host_name,
        protocol: result.protocol,
        candidates: [endpoint],
        lastSuccessfulUrl: endpoint,
        lastConnectedAt: Date.now(),
      };
      await rememberHost(host);
      setBootstrap(result);
      return host;
    },
    [rememberHost],
  );

  const addManualHostCandidate = useCallback(
    async (endpoint: string) => {
      if (!activeHost) throw new Error('Pair a host before adding another endpoint.');
      const normalized = normalizeEndpoint(endpoint);
      await rememberHost({
        ...activeHost,
        candidates: [normalized, ...activeHost.candidates.filter(value => value !== normalized)],
      });
    },
    [activeHost, rememberHost],
  );

  const removeHost = useCallback(async () => {
    if (!activeHost) return;
    const oldId = activeHost.hostId;
    if (activeHost.lastSuccessfulUrl && credential.current) {
      await fetch(`${activeHost.lastSuccessfulUrl}/mobile/device`, {
        method: 'DELETE',
        headers: {Authorization: `Bearer ${credential.current}`},
      }).catch(() => undefined);
    }
    await removeDeviceCredential(oldId);
    await removeCachedHost(oldId);
    await cleanupAppearanceBackgrounds(oldId);
    credential.current = null;
    const remaining = hosts.filter(host => host.hostId !== oldId);
    setHosts(remaining);
    setActiveHostState(remaining[0] ?? null);
    setBootstrap(null);
  }, [activeHost, hosts]);

  const setDraft = useCallback(
    (text: string) => {
      setDraftState(text);
      if (activeHost && selectedThreadId) {
        void persistDraft(activeHost.hostId, selectedThreadId, text);
      }
    },
    [activeHost, selectedThreadId],
  );

  const execute = useCallback(
    async (command: ControlCommandV1): Promise<ControlCommandResultV1> => {
      if (!activeHost?.lastSuccessfulUrl || !credential.current || status !== 'online') {
        throw new Error('The desktop is offline. Prompts and decisions stay local until you reconnect.');
      }
      let result: ControlCommandResultV1;
      try {
        result = await sendCommand(
          activeHost.lastSuccessfulUrl,
          credential.current,
          command,
        );
      } catch (error) {
        setPendingRetry(command);
        throw error;
      }
      setPendingRetry(null);
      if (result.status === 'failed' || result.status === 'conflict') {
        throw new Error(result.message ?? result.status);
      }
      await refreshBootstrap();
      await refreshTimeline('after');
      return result;
    },
    [activeHost, refreshBootstrap, refreshTimeline, status],
  );

  const command = useCallback(
    (
      kind: ControlCommandV1['kind'],
      payload: JsonValue = null,
      threadId = selectedThreadId,
      expectedRevision?: number,
    ) =>
      execute({
        command_id: newCommandId(),
        kind,
        ...(threadId ? {thread_id: threadId} : {}),
        ...(expectedRevision !== undefined ? {expected_revision: expectedRevision} : {}),
        payload,
      }),
    [execute, selectedThreadId],
  );

  const retryPendingCommand = useCallback(async () => {
    if (!pendingRetry) return null;
    return execute(pendingRetry);
  }, [execute, pendingRetry]);

  return {
    hosts,
    activeHost,
    bootstrap,
    selectedThreadId,
    timeline,
    draft,
    status,
    lastError,
    pendingRetry,
    appearanceBackgroundUri,
    setActiveHost,
    setSelectedThreadId,
    setDraft,
    pair,
    addManualHostCandidate,
    removeHost,
    refreshBootstrap,
    refreshTimeline,
    execute,
    command,
    retryPendingCommand,
  };
}
