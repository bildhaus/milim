import React, {useState} from 'react';
import {Alert, Image, Pressable, ScrollView, TextInput, View} from 'react-native';
import type {MilimController} from '../controller/useMilimController';
import {friendlyEndpoint, lowercaseMilimBrand, relativeConnectionTime} from '../mobileUi';
import {MilimIcon} from '../ui/MilimIcon';
import {useAppTheme, Text} from '../ui/appTheme';
import {MotionPressable} from '../ui/motion';
import {Button, IconButton, ActionSheetButton} from '../ui/controls';
import {showError} from '../ui/dialogs';
import {PickerSheetFrame} from '../ui/PickerSheetFrame';
import {milimLogo} from '../ui/assets';

export type HostsController = Pick<MilimController,
  | 'activeHost'
  | 'addManualHostCandidate'
  | 'hosts'
  | 'removeHost'
  | 'setActiveHost'
  | 'status'
>;

export const HostsScreen = React.memo(function MemoizedHostsScreen({
  controller,
  onPair,
  onBack,
}: {
  controller: HostsController;
  onPair: () => void;
  onBack: () => void;
}) {
  const {palette, styles} = useAppTheme();
  const [manual, setManual] = useState('');
  const [manualOpen, setManualOpen] = useState(false);
  const [detailsHostId, setDetailsHostId] = useState<string | null>(null);
  const activeHost = controller.activeHost;
  const otherHosts = controller.hosts.filter(host => host.hostId !== activeHost?.hostId);
  const detailsHost = controller.hosts.find(host => host.hostId === detailsHostId) ?? null;
  const compatible = Boolean(
    activeHost && activeHost.protocol.min <= 1 && activeHost.protocol.max >= 1,
  );
  return (
    <View style={styles.screen}>
      <ScrollView contentContainerStyle={styles.hostList}>
        <View style={styles.pageHeadingRow}>
          <IconButton icon="arrow-left" label="Back to chat" onPress={onBack} />
          <View style={styles.pageHeadingCopy}>
            <Text style={styles.eyebrow}>DIRECT CONNECTIONS</Text>
            <Text style={styles.screenTitle} accessibilityRole="header">Desktop hosts</Text>
          </View>
          <MotionPressable style={styles.hostPairAction} onPress={onPair} accessibilityLabel="Pair another desktop">
            <MilimIcon name="plus" size={14} color={palette.secondary} />
            <Text style={styles.hostPairActionText}>Pair</Text>
          </MotionPressable>
        </View>

        {activeHost ? (
          <View style={styles.hostPrimaryCard}>
            <View style={styles.hostPrimaryTopline}>
              <Image source={milimLogo} style={styles.hostMark} />
              <View style={styles.hostPrimaryBody}>
                <Text style={styles.hostName} numberOfLines={1}>{lowercaseMilimBrand(activeHost.displayName)}</Text>
                <Text style={styles.hostEndpoint} numberOfLines={1}>{friendlyEndpoint(activeHost.lastSuccessfulUrl)}</Text>
              </View>
              <View style={[styles.hostStatusPill, controller.status === 'online' && styles.hostStatusPillOnline]}>
                <View style={[styles.dot, controller.status === 'online' && styles.dotOnline]} />
                <Text style={styles.hostStatusText}>{controller.status}</Text>
              </View>
              <MotionPressable
                style={styles.hostMenu}
                onPress={() => setDetailsHostId(activeHost.hostId)}
                accessibilityLabel={`Details for ${lowercaseMilimBrand(activeHost.displayName)}`}>
                <MilimIcon name="more-horizontal" size={17} color={palette.muted} />
              </MotionPressable>
            </View>
            <View style={styles.hostPrimaryFooter}>
              <Text style={styles.hostActivity}>{relativeConnectionTime(activeHost.lastConnectedAt)}</Text>
              <View style={styles.hostCompatibility}>
                <MilimIcon name={compatible ? 'check' : 'x'} size={12} color={compatible ? palette.success : palette.danger} />
                <Text style={styles.hostCompatibilityText}>Protocol v1 {compatible ? 'compatible' : 'unsupported'}</Text>
              </View>
            </View>
          </View>
        ) : null}

        {otherHosts.length ? (
          <View style={styles.hostSection}>
            <Text style={styles.hostSectionLabel}>SAVED DESKTOPS</Text>
            <View style={styles.hostRows}>
              {otherHosts.map(host => (
                <Pressable key={host.hostId} style={styles.hostRow} onPress={() => controller.setActiveHost(host.hostId)}>
                  <Image source={milimLogo} style={styles.hostRowMark} />
                  <View style={styles.hostRowBody}>
                    <Text style={styles.hostRowTitle} numberOfLines={1}>{lowercaseMilimBrand(host.displayName)}</Text>
                    <Text style={styles.hostRowMeta} numberOfLines={1}>{relativeConnectionTime(host.lastConnectedAt)}</Text>
                  </View>
                  <Text style={styles.hostSwitchText}>Switch</Text>
                </Pressable>
              ))}
            </View>
          </View>
        ) : null}

        <View style={styles.hostSection}>
          <MotionPressable
            style={styles.hostDisclosure}
            onPress={() => setManualOpen(value => !value)}
            accessibilityLabel={`${manualOpen ? 'Close' : 'Open'} manual connection form`}>
            <View style={styles.hostDisclosureIcon}>
              <MilimIcon name="link" size={15} color={palette.secondary} />
            </View>
            <View style={styles.hostDisclosureBody}>
              <Text style={styles.hostDisclosureTitle}>Connect manually</Text>
              <Text style={styles.hostDisclosureCopy}>Add another address for this desktop</Text>
            </View>
            <MilimIcon name={manualOpen ? 'chevron-up' : 'chevron-down'} size={14} color={palette.muted} />
          </MotionPressable>
          {manualOpen ? (
            <View style={styles.hostManualForm}>
              <Text style={styles.help}>Tailscale HTTPS is recommended. Use plain HTTP only on a trusted LAN.</Text>
              <TextInput
                style={styles.input}
                value={manual}
                onChangeText={setManual}
                placeholder="https://desktop.tailnet.ts.net:10000"
                accessibilityLabel="Desktop URL"
                placeholderTextColor={palette.placeholder}
                autoCapitalize="none"
                autoCorrect={false}
                keyboardType="url"
              />
              <View style={styles.hostManualActions}>
                <Button
                  label="Save address"
                  icon="check"
                  tone="quiet"
                  disabled={!manual.trim()}
                  onPress={() => void controller.addManualHostCandidate(manual).then(() => {
                    setManual('');
                    setManualOpen(false);
                  }).catch(showError)}
                />
              </View>
            </View>
          ) : null}
        </View>

        <View style={styles.hostInfoRow}>
          <View style={styles.hostDisclosureIcon}>
            <MilimIcon name="info" size={15} color={palette.secondary} />
          </View>
          <View style={styles.hostDisclosureBody}>
            <Text style={styles.hostDisclosureTitle}>Foreground catch-up</Text>
            <Text style={styles.hostDisclosureCopy}>milim refreshes when reopened; background push is not enabled in v1.</Text>
          </View>
        </View>
      </ScrollView>

      <PickerSheetFrame
        visible={Boolean(detailsHost)}
        title={detailsHost ? lowercaseMilimBrand(detailsHost.displayName) : 'Desktop details'}
        subtitle="Paired desktop"
        compact
        onClose={() => setDetailsHostId(null)}>
        <View style={styles.hostDetails}>
          <View style={styles.hostDetailBlock}>
            <Text style={styles.drawerProjectLabel}>CONNECTION ADDRESS</Text>
            <Text style={styles.hostDetailValue} selectable>{detailsHost?.lastSuccessfulUrl ?? 'No reachable endpoint'}</Text>
          </View>
          <View style={styles.hostDetailGrid}>
            <View style={styles.hostDetailStat}>
              <Text style={styles.drawerProjectLabel}>PROTOCOL</Text>
              <Text style={styles.hostDetailStatValue}>{detailsHost ? `${detailsHost.protocol.min}–${detailsHost.protocol.max}` : '—'}</Text>
            </View>
            <View style={styles.hostDetailStat}>
              <Text style={styles.drawerProjectLabel}>LAST CONNECTION</Text>
              <Text style={styles.hostDetailStatValue}>{detailsHost?.lastConnectedAt ? new Date(detailsHost.lastConnectedAt).toLocaleString() : 'Never'}</Text>
            </View>
          </View>
          {detailsHost && detailsHost.hostId !== activeHost?.hostId ? (
            <ActionSheetButton icon="check" label="Use this desktop" onPress={() => {
              controller.setActiveHost(detailsHost.hostId);
              setDetailsHostId(null);
            }} />
          ) : null}
          {detailsHost?.hostId === activeHost?.hostId ? (
            <ActionSheetButton icon="trash" label="Revoke this phone" danger onPress={() => Alert.alert(
              'Remove host?',
              'This removes the local cache and asks the desktop to revoke this phone.',
              [
                {text: 'Cancel', style: 'cancel'},
                {text: 'Remove', style: 'destructive', onPress: () => {
                  setDetailsHostId(null);
                  void controller.removeHost().catch(showError);
                }},
              ],
            )} />
          ) : null}
        </View>
      </PickerSheetFrame>
    </View>
  );
});
