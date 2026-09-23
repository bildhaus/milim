import {useCallback, useEffect, useRef, useState} from 'react';
import {ActivityIndicator, Image, Modal, PermissionsAndroid, Platform, ScrollView, StyleSheet, TextInput, View} from 'react-native';
import {SafeAreaView} from 'react-native-safe-area-context';
import {type NearbyPairingStage} from '../controller/useMilimController';
import {discoverMilimHosts, filterPairableMilimHosts, type DiscoveredHost} from '../discovery';
import {friendlyEndpoint, friendlyPairingError, lowercaseMilimBrand} from '../mobileUi';
import {MilimIcon} from '../ui/MilimIcon';
import {useAppTheme, Text} from '../ui/appTheme';
import {MotionPressable} from '../ui/motion';
import {Button, IconButton} from '../ui/controls';
import {milimLogo} from '../ui/assets';

export function PairingScreen({
  claim,
  setClaim,
  onPair,
  onPairNearby,
  pairedHostIds,
  additional = false,
  onClose,
}: {
  claim: string;
  setClaim: (value: string) => void;
  onPair: (claim: string, name: string) => Promise<unknown>;
  onPairNearby: (
    host: DiscoveredHost,
    deviceName: string,
    signal: AbortSignal,
    onStage: (stage: NearbyPairingStage) => void,
  ) => Promise<unknown>;
  pairedHostIds: readonly string[];
  additional?: boolean;
  onClose?: () => void;
}) {
  const {palette, styles} = useAppTheme();
  const [deviceName, setDeviceName] = useState(`${Platform.OS === 'ios' ? 'iPhone' : 'Android'} controller`);
  const [scanner, setScanner] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nearbyHosts, setNearbyHosts] = useState<DiscoveredHost[]>([]);
  const [discovering, setDiscovering] = useState(true);
  const [discoveryError, setDiscoveryError] = useState<string | null>(null);
  const [showManualPairing, setShowManualPairing] = useState(Boolean(claim));
  const [nearbyPairing, setNearbyPairing] = useState<{
    host: DiscoveredHost;
    stage: NearbyPairingStage;
  } | null>(null);
  const discoveryVersion = useRef(0);
  const discoveryActive = useRef(false);
  const nearbyPairingAbort = useRef<AbortController | null>(null);
  const refreshNearbyHosts = useCallback(async () => {
    if (discoveryActive.current) return;
    discoveryActive.current = true;
    const version = ++discoveryVersion.current;
    setDiscovering(true);
    setDiscoveryError(null);
    try {
      const hosts = filterPairableMilimHosts(
        await discoverMilimHosts(),
        pairedHostIds,
      );
      if (version === discoveryVersion.current) setNearbyHosts(hosts);
    } catch (reason) {
      if (version === discoveryVersion.current) {
        setDiscoveryError(reason instanceof Error ? reason.message : String(reason));
      }
    } finally {
      if (version === discoveryVersion.current) {
        discoveryActive.current = false;
        setDiscovering(false);
      }
    }
  }, [pairedHostIds]);
  useEffect(() => {
    void refreshNearbyHosts();
    const timer = setInterval(() => void refreshNearbyHosts(), 15_000);
    return () => {
      clearInterval(timer);
      discoveryVersion.current += 1;
      discoveryActive.current = false;
    };
  }, [refreshNearbyHosts]);
  useEffect(() => {
    if (claim.trim()) setShowManualPairing(true);
  }, [claim]);
  useEffect(() => () => nearbyPairingAbort.current?.abort(), []);

  const connectNearby = async (host: DiscoveredHost) => {
    nearbyPairingAbort.current?.abort();
    const controller = new AbortController();
    nearbyPairingAbort.current = controller;
    setError(null);
    setNearbyPairing({host, stage: 'requesting'});
    try {
      await onPairNearby(host, deviceName, controller.signal, stage => {
        setNearbyPairing(current => current?.host.endpoint === host.endpoint
          ? {...current, stage}
          : current);
      });
    } catch (reason) {
      if (!controller.signal.aborted) {
        setError(friendlyPairingError(reason));
      }
    } finally {
      if (nearbyPairingAbort.current === controller) {
        nearbyPairingAbort.current = null;
        setNearbyPairing(null);
      }
    }
  };

  const cancelNearbyPairing = () => {
    nearbyPairingAbort.current?.abort();
    nearbyPairingAbort.current = null;
    setNearbyPairing(null);
  };
  const pair = async () => {
    setBusy(true);
    setError(null);
    try {
      await onPair(claim, deviceName);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };
  return (
    <SafeAreaView style={styles.onboarding}>
      <View style={styles.onboardingTopbar}>
        <View style={styles.onboardingBrand}>
          <Image source={milimLogo} style={styles.onboardingMark} />
          <Text style={styles.onboardingWordmark}>milim</Text>
        </View>
        {onClose ? (
          <IconButton icon="x" label="Close pairing" onPress={onClose} />
        ) : (
          <Text style={styles.mobileLabel}>MOBILE</Text>
        )}
      </View>
      <ScrollView
        contentContainerStyle={styles.onboardingContent}
        keyboardShouldPersistTaps="handled">
        <View style={styles.onboardingIntro}>
          <Text style={styles.eyebrow}>{additional ? 'PAIR ANOTHER DESKTOP' : 'PAIR A DESKTOP'}</Text>
          <Text style={styles.heroTitle} accessibilityRole="header">
            {additional ? 'Connect another milim workbench.' : 'Your milim workbench, in your pocket.'}
          </Text>
          <Text style={styles.heroCopy}>
            {additional
              ? 'Choose a nearby desktop, then approve the request there. Each desktop stays authoritative.'
              : 'Control the same threads, runs, queues, and approvals. Your desktop stays authoritative.'}
          </Text>
        </View>
        <View style={styles.nearbyPanel}>
          <View style={styles.nearbyHeading}>
            <View style={styles.flex}>
              <Text style={styles.nearbyTitle} accessibilityRole="header">Nearby desktops</Text>
              <Text style={styles.nearbySubtitle}>
                {discovering
                  ? 'Searching your trusted network…'
                  : nearbyHosts.length
                    ? `${nearbyHosts.length} ready to pair`
                    : additional ? 'No new desktops detected' : 'No desktops detected'}
              </Text>
            </View>
            <MotionPressable
              style={[styles.nearbyRefresh, discovering && styles.disabled]}
              onPress={() => void refreshNearbyHosts()}
              disabled={discovering}
              accessibilityLabel="Refresh nearby desktops">
              <MilimIcon name="refresh" size={14} color={palette.secondary} />
            </MotionPressable>
          </View>
          {nearbyPairing ? (
            <View style={[styles.nearbyHost, styles.nearbyHostPending]}>
              <View style={styles.nearbyPairingSpinner}>
                <ActivityIndicator size="small" color={palette.text} />
              </View>
              <View style={styles.nearbyHostBody}>
                <Text style={styles.nearbyHostName} numberOfLines={1}>
                  {nearbyPairing.stage === 'requesting'
                    ? 'Requesting access…'
                    : nearbyPairing.stage === 'connecting'
                      ? 'Connecting securely…'
                      : 'Approve on your desktop'}
                </Text>
                <Text style={styles.nearbyHostEndpoint} numberOfLines={2}>
                  {nearbyPairing.stage === 'waiting'
                    ? `${lowercaseMilimBrand(nearbyPairing.host.name)} is waiting for your confirmation.`
                    : friendlyEndpoint(nearbyPairing.host.endpoint)}
                </Text>
              </View>
              <Button label="Cancel" tone="quiet" onPress={cancelNearbyPairing} />
            </View>
          ) : nearbyHosts.map(host => (
            <MotionPressable
              key={host.hostId ?? host.endpoint}
              style={styles.nearbyHost}
              onPress={() => void connectNearby(host)}
              accessibilityLabel={`Connect to ${lowercaseMilimBrand(host.name)}`}>
              <Image source={milimLogo} style={styles.nearbyHostMark} />
              <View style={styles.nearbyHostBody}>
                <Text style={styles.nearbyHostName} numberOfLines={1}>
                  {lowercaseMilimBrand(host.name)}
                </Text>
                <Text style={styles.nearbyHostEndpoint} numberOfLines={1}>
                  {friendlyEndpoint(host.endpoint)}
                </Text>
              </View>
              <View style={styles.nearbyConnectAction}>
                <Text style={styles.nearbyConnectText}>Connect</Text>
                <MilimIcon name="chevron-right" size={13} color={palette.text} />
              </View>
            </MotionPressable>
          ))}
          {!discovering && !nearbyHosts.length ? (
            <Text style={styles.nearbyEmpty}>
              {discoveryError
                ? 'Local discovery is unavailable. You can still pair with the QR code or link below.'
                : 'Enable the companion bridge and trusted-network discovery on your desktop. Some emulators and VPNs block local discovery.'}
            </Text>
          ) : null}
          {nearbyHosts.length ? (
            <Text style={styles.nearbyHelp}>
              Tap a desktop, then approve the request there. No code or link needed.
            </Text>
          ) : null}
          {error ? <Text style={styles.formError}>{error}</Text> : null}
        </View>
        <MotionPressable
          style={styles.manualPairToggle}
          onPress={() => setShowManualPairing(current => !current)}
          accessibilityLabel="Pair with QR code or link">
          <View style={styles.manualPairIcon}>
            <MilimIcon name="link" size={16} color={palette.secondary} />
          </View>
          <View style={styles.flex}>
            <Text style={styles.manualPairTitle}>Pair another way</Text>
            <Text style={styles.manualPairCopy}>Use a QR code or link when nearby discovery is unavailable.</Text>
          </View>
          <MilimIcon
            name={showManualPairing ? 'chevron-up' : 'chevron-down'}
            size={15}
            color={palette.muted}
          />
        </MotionPressable>
        {showManualPairing ? <View style={styles.pairingPanel}>
          <View style={styles.panelHeading}>
            <View style={styles.panelIcon}>
              <MilimIcon name="smartphone" size={17} color={palette.text} />
            </View>
            <View style={styles.flex}>
              <Text style={styles.panelTitle} accessibilityRole="header">QR code or pairing link</Text>
              <Text style={styles.help}>For Tailscale, VPNs, or networks that block discovery</Text>
            </View>
          </View>
          <Text style={styles.fieldLabel}>DEVICE NAME</Text>
          <TextInput
        style={styles.input}
        value={deviceName}
        onChangeText={setDeviceName}
        placeholder="Device name"
        accessibilityLabel="Device name"
        placeholderTextColor={palette.placeholder}
      />
      <Text style={styles.fieldLabel}>PAIRING LINK</Text>
      <TextInput
        style={[styles.input, styles.claimInput]}
        value={claim}
        onChangeText={setClaim}
        placeholder="Paste the link from milim desktop"
        accessibilityLabel="Pairing link"
        placeholderTextColor={palette.placeholder}
        autoCapitalize="none"
        autoCorrect={false}
        multiline
      />
      <View style={styles.pairActions}>
        <Button label="Scan QR" icon="scan" tone="quiet" onPress={() => setScanner(true)} />
        <Button label={busy ? 'Pairing…' : 'Pair desktop'} icon="arrow-up" onPress={() => void pair()} disabled={busy || !claim.trim()} />
      </View>
        </View> : null}
        <View style={styles.directNote}>
          <View style={[styles.dot, styles.dotOnline]} />
          <Text style={styles.directNoteText}>
            Direct over Tailscale or trusted LAN. No milim account, cloud relay, or hosted transcript store.
          </Text>
        </View>
      </ScrollView>
      <Scanner visible={scanner} onClose={() => setScanner(false)} onRead={value => {
        setClaim(value);
        setScanner(false);
      }} />
    </SafeAreaView>
  );
}

export function PairingModal({
  visible,
  claim,
  setClaim,
  onClose,
  onPair,
  onPairNearby,
  pairedHostIds,
}: {
  visible: boolean;
  claim: string;
  setClaim: (value: string) => void;
  onClose: () => void;
  onPair: (claim: string, deviceName: string) => Promise<unknown>;
  onPairNearby: (
    host: DiscoveredHost,
    deviceName: string,
    signal: AbortSignal,
    onStage: (stage: NearbyPairingStage) => void,
  ) => Promise<unknown>;
  pairedHostIds: readonly string[];
}) {
  return (
    <Modal visible={visible} animationType="slide" presentationStyle="pageSheet" onRequestClose={onClose}>
      {visible ? (
        <PairingScreen
          claim={claim}
          setClaim={setClaim}
          onPair={onPair}
          onPairNearby={onPairNearby}
          pairedHostIds={pairedHostIds}
          additional
          onClose={onClose}
        />
      ) : null}
    </Modal>
  );
}

export function Scanner({visible, onClose, onRead}: {visible: boolean; onClose: () => void; onRead: (value: string) => void}) {
  const {palette, styles} = useAppTheme();
  const [allowed, setAllowed] = useState(Platform.OS === 'ios');
  useEffect(() => {
    if (!visible || Platform.OS !== 'android') return;
    void PermissionsAndroid.request(PermissionsAndroid.PERMISSIONS.CAMERA).then(result => {
      setAllowed(result === PermissionsAndroid.RESULTS.GRANTED);
    });
  }, [visible]);
  if (!visible) return null;
  const CameraView = require('react-native-camera-kit').Camera as typeof import('react-native-camera-kit').Camera;
  return (
    <Modal visible={visible} animationType="fade" onRequestClose={onClose}>
      <View style={styles.scanner}>
        {allowed ? (
          <CameraView
            style={StyleSheet.absoluteFill}
            scanBarcode
            showFrame
            frameColor={palette.accent}
            laserColor={palette.accent}
            onReadCode={event => onRead(event.nativeEvent.codeStringValue)}
          />
        ) : (
          <Text style={styles.formError}>Camera permission is needed only to scan the pairing QR.</Text>
        )}
        <SafeAreaView style={styles.scannerOverlay}>
          <Text style={styles.scannerTitle}>Scan milim pairing QR</Text>
          <Button label="Close" tone="quiet" onPress={onClose} />
        </SafeAreaView>
      </View>
    </Modal>
  );
}
