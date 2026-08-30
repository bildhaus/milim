import {useCallback, useEffect, useRef, useState} from 'react';
import {AppState, Platform, type AppStateStatus} from 'react-native';
import {
  cleanupAppearanceBackgrounds,
  fetchAppearanceBackground,
} from '../appearance';
import {prepareWireAttachments} from '../attachments';
import {
  cancelPairingRequest,
  claimPairing,
  claimPairingRequest,
  connectControlSocket,
  createPairingRequest,
  fetchBootstrap,
  fetchMobileHostProbe,
  fetchPairingRequestStatus,
  fetchRunEvents,
  fetchRunInspection,
  fetchTimeline,
  newCommandId,
  normalizeEndpoint,
  sendCommand,
} from '../control/client';
import {
  applyControlEvents,
  applyTimelinePage,
  controlEventInvalidatesBootstrap,
  emptyReplica,
  type TimelineReplica,
} from '../control/replica';
import type {
  ControlBootstrapV1,
  ControlCommandResultV1,
  ControlCommandV1,
  ControlEventV1,
  ControlAttachmentV1,
  JsonValue,
  SavedHost,
} from '../control/types';
import {isProtocolCompatible} from '../control/types';
import type {RunEventPageV1, RunInspectionV1} from '../control/generated-v1';
import {discoverMilimHosts} from '../discovery';
import type {DiscoveredHost} from '../discovery';
import {friendlyConnectionError} from '../mobileUi';
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
import {mobilePerfMark, mobilePerfMeasure} from '../performance';

export type ConnectionStatus = 'offline' | 'connecting' | 'online' | 'incompatible';
export type NearbyPairingStage = 'requesting' | 'waiting' | 'connecting';

function pairingCancelledError(): Error {
  const error = new Error('Pairing cancelled.');
  error.name = 'AbortError';
  return error;
}

function waitForPairingPoll(signal: AbortSignal, timeoutMs = 900): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(pairingCancelledError());
      return;
    }
    const timeout = setTimeout(done, timeoutMs);
    const onAbort = () => done(pairingCancelledError());
    function done(error?: Error) {
      clearTimeout(timeout);
      signal.removeEventListener('abort', onAbort);
      if (error) reject(error);
      else resolve();
    }
    signal.addEventListener('abort', onAbort, {once: true});
  });
}

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
  const [connectionRevision, setConnectionRevision] = useState(0);
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
  const bootstrapRefreshPromise = useRef<Promise<ControlBootstrapV1 | null> | null>(null);
  const timelineRefreshPromises = useRef<Partial<Record<'tail' | 'after' | 'before', Promise<void>>>>({});
  const eventBuffer = useRef<ControlEventV1[]>([]);
  const eventFlushFrame = useRef<number | null>(null);
  const reconciliationTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const draftPersistTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingDraft = useRef<{hostId: string; threadId: string; text: string} | null>(null);
  const draftPersistChain = useRef<Promise<void>>(Promise.resolve());
  const timelineCacheTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingTimelineCache = useRef<{hostId: string; replica: TimelineReplica} | null>(null);
  const timelineCacheChain = useRef<Promise<void>>(Promise.resolve());
  const lastTimelineCacheKey = useRef('');

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

  const flushDraftPersistence = useCallback(async () => {
    if (draftPersistTimer.current) clearTimeout(draftPersistTimer.current);
    draftPersistTimer.current = null;
    const pending = pendingDraft.current;
    pendingDraft.current = null;
    if (!pending) return;
    const operation = draftPersistChain.current.catch(() => {}).then(async () => {
      mobilePerfMark('draft.persist.start');
      await persistDraft(pending.hostId, pending.threadId, pending.text);
      mobilePerfMark('draft.persist.end');
      mobilePerfMeasure('draft.persist', 'draft.persist.start', 'draft.persist.end');
    });
    draftPersistChain.current = operation;
    try {
      await operation;
    } catch {
      // The in-memory draft remains authoritative until the next flush.
    }
  }, []);

  const flushTimelineCache = useCallback(async () => {
    if (timelineCacheTimer.current) clearTimeout(timelineCacheTimer.current);
    timelineCacheTimer.current = null;
    const pending = pendingTimelineCache.current;
    pendingTimelineCache.current = null;
    if (!pending) return;
    const key = `${pending.hostId}:${pending.replica.threadId}:${pending.replica.epoch}:${pending.replica.lastSeq ?? ''}`;
    if (key === lastTimelineCacheKey.current) return;
    const operation = timelineCacheChain.current.catch(() => {}).then(async () => {
      if (key === lastTimelineCacheKey.current) return;
      await saveTimelineTail(pending.hostId, pageFromReplica(pending.replica));
      lastTimelineCacheKey.current = key;
    });
    timelineCacheChain.current = operation;
    try {
      await operation;
    } catch {
      // A later authoritative tail refresh can repopulate this bounded cache.
    }
  }, []);

  const queueTimelineCache = useCallback((hostId: string, replica: TimelineReplica) => {
    pendingTimelineCache.current = {hostId, replica};
    if (timelineCacheTimer.current) clearTimeout(timelineCacheTimer.current);
    timelineCacheTimer.current = setTimeout(() => void flushTimelineCache(), 500);
  }, [flushTimelineCache]);

  const refreshBootstrap = useCallback((): Promise<ControlBootstrapV1 | null> => {
    if (bootstrapRefreshPromise.current) return bootstrapRefreshPromise.current;
    const request = (async () => {
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
    })();
    bootstrapRefreshPromise.current = request;
    void request.finally(() => {
      if (bootstrapRefreshPromise.current === request) bootstrapRefreshPromise.current = null;
    });
    return request;
  }, []);

  const refreshTimeline = useCallback(
    (mode: 'tail' | 'after' | 'before' = 'after'): Promise<void> => {
      const existing = timelineRefreshPromises.current[mode];
      if (existing) return existing;
      const request = (async () => {
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
        if (selectedThreadRef.current !== threadId || activeHostRef.current?.hostId !== host.hostId) return;
        setTimeline(next);
        timelineRef.current = next;
        queueTimelineCache(host.hostId, next);
      })();
      timelineRefreshPromises.current[mode] = request;
      void request.finally(() => {
        if (timelineRefreshPromises.current[mode] === request) {
          delete timelineRefreshPromises.current[mode];
        }
      });
      return request;
    },
    [queueTimelineCache],
  );

  const queueControlEvent = useCallback((event: ControlEventV1, expectedHostId: string) => {
    eventBuffer.current.push(event);
    if (eventFlushFrame.current !== null) return;
    eventFlushFrame.current = requestAnimationFrame(() => {
      eventFlushFrame.current = null;
      const events = eventBuffer.current.splice(0);
      const current = timelineRef.current;
      if (!current || !events.length) return;
      mobilePerfMark('timeline.flush.start');
      const next = applyControlEvents(current, events, expectedHostId);
      if (next !== current) {
        timelineRef.current = next;
        setTimeline(next);
      }
      mobilePerfMark('timeline.flush.end');
      mobilePerfMeasure('timeline.flush', 'timeline.flush.start', 'timeline.flush.end');
      if (next.needsTailRefresh && !current.needsTailRefresh) void refreshTimeline('tail');
    });
  }, [refreshTimeline]);

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
            queueControlEvent(event, host.hostId);
            if (controlEventInvalidatesBootstrap(event.type)) {
              if (bootstrapRefreshTimer.current) clearTimeout(bootstrapRefreshTimer.current);
              bootstrapRefreshTimer.current = setTimeout(() => void refreshBootstrap(), 250);
            }
          },
          () => {
            if (!cancelled && AppState.currentState === 'active') {
              setStatus('offline');
              setLastError(friendlyConnectionError(
                [endpoint],
                new Error('The connection closed unexpectedly.'),
              ));
            }
          },
        );
      })
      .catch(error => {
        if (!cancelled) {
          setStatus('offline');
          setLastError(friendlyConnectionError(
            [host.lastSuccessfulUrl, ...host.candidates],
            error,
          ));
        }
      });
    return () => {
      cancelled = true;
      socket.current?.close();
      socket.current = null;
      eventBuffer.current = [];
      if (eventFlushFrame.current !== null) cancelAnimationFrame(eventFlushFrame.current);
      eventFlushFrame.current = null;
    };
  }, [activeHost?.hostId, appState, connectionRevision, queueControlEvent, rememberHost, refreshBootstrap, tryBootstrap]);

  const reconnect = useCallback(() => {
    setLastError(null);
    setConnectionRevision(current => current + 1);
  }, []);

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

  const finishPairing = useCallback(
    async (endpoint: string, hostId: string, paired: {device_key: string}) => {
      const result = await fetchBootstrap(endpoint, paired.device_key);
      assertHostIdentity(hostId, result.host_id);
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
      return finishPairing(endpoint, claim.hostId, paired);
    },
    [finishPairing],
  );

  const pairNearby = useCallback(
    async (
      discoveredHost: DiscoveredHost,
      deviceName: string,
      signal: AbortSignal,
      onStage: (stage: NearbyPairingStage) => void,
    ) => {
      const endpoint = normalizeEndpoint(discoveredHost.endpoint);
      onStage('requesting');
      const probe = await fetchMobileHostProbe(endpoint, signal);
      if (probe.service !== 'milim-mobile-control') {
        throw new Error('This address is not a milim desktop.');
      }
      if (discoveredHost.hostId) assertHostIdentity(discoveredHost.hostId, probe.host_id);
      const request = await createPairingRequest(
        endpoint,
        deviceName,
        Platform.OS === 'ios' ? 'ios' : 'android',
        signal,
      );
      let claimed = false;
      try {
        onStage('waiting');
        while (!signal.aborted) {
          const requestStatus = await fetchPairingRequestStatus(
            endpoint,
            request.request_id,
            request.request_key,
            signal,
          );
          if (requestStatus.status === 'denied') {
            throw new Error('Connection declined on the desktop.');
          }
          if (requestStatus.status === 'approved' || requestStatus.status === 'paired') {
            onStage('connecting');
            const paired = await claimPairingRequest(
              endpoint,
              request.request_id,
              request.request_key,
              signal,
            );
            claimed = true;
            return await finishPairing(endpoint, probe.host_id, paired);
          }
          if (Date.now() >= requestStatus.expires_at * 1000) {
            throw new Error('The desktop did not approve this request in time. Try again.');
          }
          await waitForPairingPoll(signal);
        }
        throw pairingCancelledError();
      } finally {
        if (!claimed) {
          void cancelPairingRequest(
            endpoint,
            request.request_id,
            request.request_key,
          ).catch(() => {});
        }
      }
    },
    [finishPairing],
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
      const hostId = activeHostRef.current?.hostId;
      const threadId = selectedThreadRef.current;
      if (hostId && threadId) {
        pendingDraft.current = {hostId, threadId, text};
        if (draftPersistTimer.current) clearTimeout(draftPersistTimer.current);
        if (!text) void flushDraftPersistence();
        else draftPersistTimer.current = setTimeout(() => void flushDraftPersistence(), 300);
      }
    },
    [flushDraftPersistence],
  );

  useEffect(() => () => {
    void flushDraftPersistence();
  }, [activeHost?.hostId, flushDraftPersistence, selectedThreadId]);

  useEffect(() => {
    if (appState !== 'active') {
      void flushDraftPersistence();
      void flushTimelineCache();
    }
  }, [appState, flushDraftPersistence, flushTimelineCache]);

  useEffect(() => () => {
    if (bootstrapRefreshTimer.current) clearTimeout(bootstrapRefreshTimer.current);
    if (reconciliationTimer.current) clearTimeout(reconciliationTimer.current);
    if (draftPersistTimer.current) clearTimeout(draftPersistTimer.current);
    if (timelineCacheTimer.current) clearTimeout(timelineCacheTimer.current);
    if (eventFlushFrame.current !== null) cancelAnimationFrame(eventFlushFrame.current);
    void flushDraftPersistence();
    void flushTimelineCache();
  }, [flushDraftPersistence, flushTimelineCache]);

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
      if (reconciliationTimer.current) clearTimeout(reconciliationTimer.current);
      reconciliationTimer.current = setTimeout(() => {
        reconciliationTimer.current = null;
        void Promise.allSettled([refreshBootstrap(), refreshTimeline('after')]);
      }, 250);
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

  const loadRunDetails = useCallback(
    async (runId: string): Promise<{inspection: RunInspectionV1; events: RunEventPageV1}> => {
      if (!activeHost?.lastSuccessfulUrl || !credential.current || status !== 'online') {
        throw new Error('Reconnect to the desktop to inspect this run.');
      }
      const [inspection, events] = await Promise.all([
        fetchRunInspection(activeHost.lastSuccessfulUrl, credential.current, runId),
        fetchRunEvents(activeHost.lastSuccessfulUrl, credential.current, runId),
      ]);
      return {inspection, events};
    },
    [activeHost, status],
  );

  const loadMoreRunEvents = useCallback(
    async (runId: string, afterSeq: number): Promise<RunEventPageV1> => {
      if (!activeHost?.lastSuccessfulUrl || !credential.current || status !== 'online') {
        throw new Error('Reconnect to the desktop to inspect this run.');
      }
      return fetchRunEvents(activeHost.lastSuccessfulUrl, credential.current, runId, afterSeq);
    },
    [activeHost, status],
  );

  const prepareAttachments = useCallback(
    async (attachments: ControlAttachmentV1[]) => {
      if (!activeHost?.lastSuccessfulUrl || !credential.current || status !== 'online') {
        throw new Error('Reconnect to the desktop before uploading attachments.');
      }
      return prepareWireAttachments(attachments, {
        endpoint: activeHost.lastSuccessfulUrl,
        deviceKey: credential.current,
        uploads: bootstrap?.capabilities.attachment_uploads === true,
      });
    },
    [activeHost, bootstrap?.capabilities.attachment_uploads, status],
  );

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
    reconnect,
    pair,
    pairNearby,
    addManualHostCandidate,
    removeHost,
    refreshBootstrap,
    refreshTimeline,
    execute,
    command,
    loadRunDetails,
    loadMoreRunEvents,
    prepareAttachments,
    retryPendingCommand,
  };
}
