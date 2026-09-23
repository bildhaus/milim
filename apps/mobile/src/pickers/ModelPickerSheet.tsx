import {useCallback, useEffect, useMemo, useState} from 'react';
import {FlatList, ScrollView, TextInput, View} from 'react-native';
import {type JsonValue} from '../control/types';
import {DEFAULT_MODEL_PICKER_PREFERENCES, modelPickerFavoriteIds, modelPickerGroups, toggledModelFavoriteIds, type MobileModelCapability, type MobileModelOption} from '../modelPicker';
import {readModelPickerPreferences, saveModelPickerPreferences} from '../storage/cache';
import {MilimIcon, type MilimIconName} from '../ui/MilimIcon';
import {ProviderIcon} from '../ui/ProviderIcon';
import {useAppTheme, Text} from '../ui/appTheme';
import {MotionPressable} from '../ui/motion';
import {Empty} from '../ui/controls';
import {showError} from '../ui/dialogs';
import {PickerSheetFrame} from '../ui/PickerSheetFrame';

export const capabilityIcons: Record<MobileModelCapability, MilimIconName> = {
  vision: 'eye',
  tools: 'plug',
  reasoning: 'sparkles',
  fast: 'bolt',
  image: 'image',
  video: 'video',
  music: 'volume',
};

export const reasoningEffortLabels: Record<string, string> = {
  auto: 'Auto',
  none: 'Off',
  minimal: 'Minimal',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  on: 'On',
  xhigh: 'X-high',
  max: 'Max',
};

export function ModelPickerSheet({
  visible,
  hostId,
  models,
  favoriteIds,
  selectedId,
  reasoningEffortOverrides,
  onClose,
  onFavoriteIdsChange,
  onSelect,
}: {
  visible: boolean;
  hostId: string;
  models: JsonValue[];
  favoriteIds?: string[];
  selectedId: string | null;
  reasoningEffortOverrides?: Record<string, string>;
  onClose: () => void;
  onFavoriteIdsChange?: (favoriteModelIds: string[]) => Promise<void>;
  onSelect: (id: string, reasoningEffort?: string) => void;
}) {
  const {palette, styles} = useAppTheme();
  const [query, setQuery] = useState('');
  const [effortModelId, setEffortModelId] = useState<string | null>(null);
  const [preferences, setPreferences] = useState(DEFAULT_MODEL_PICKER_PREFERENCES);
  const [optimisticFavoriteIds, setOptimisticFavoriteIds] = useState<string[] | null>(null);
  const effectiveFavoriteIds = modelPickerFavoriteIds(
    optimisticFavoriteIds ?? favoriteIds,
    preferences.favorites,
  );
  const groups = useMemo(
    () => modelPickerGroups(models, query, effectiveFavoriteIds, preferences.favoritesOnly),
    [effectiveFavoriteIds, models, preferences.favoritesOnly, query],
  );
  const collapsedGroups = useMemo(
    () => new Set(preferences.collapsedGroups),
    [preferences.collapsedGroups],
  );
  const filtering = Boolean(query.trim()) || preferences.favoritesOnly;
  const rows = useMemo(
    () => groups.flatMap(group => {
      const collapsible = group.title !== 'Favorites' && !filtering;
      const collapsed = collapsible && collapsedGroups.has(group.title);
      return [
        {
          type: 'header' as const,
          key: `header:${group.title}`,
          title: group.title,
          count: group.models.length,
          brand: group.models[0]?.brand ?? null,
          collapsible,
          collapsed,
        },
        ...(collapsed ? [] : group.models.flatMap(model => [
          {type: 'model' as const, key: `${group.title}:${model.id}`, model},
          ...(effortModelId === model.id
            ? [{type: 'effort' as const, key: `${group.title}:${model.id}:effort`, model}]
            : []),
        ])),
      ];
    }),
    [collapsedGroups, effortModelId, filtering, groups],
  );
  useEffect(() => {
    if (!visible) {
      setQuery('');
      setEffortModelId(null);
      return;
    }
    setPreferences(DEFAULT_MODEL_PICKER_PREFERENCES);
    setOptimisticFavoriteIds(null);
    if (!hostId) return;
    let cancelled = false;
    void readModelPickerPreferences(hostId)
      .then(next => {
        if (!cancelled) setPreferences(next);
      })
      .catch(showError);
    return () => {
      cancelled = true;
    };
  }, [hostId, visible]);
  useEffect(() => {
    setOptimisticFavoriteIds(null);
  }, [favoriteIds]);
  const updatePreferences = useCallback((next: typeof preferences) => {
    setPreferences(next);
    if (hostId) void saveModelPickerPreferences(hostId, next).catch(showError);
  }, [hostId]);
  const toggleFavorite = useCallback((favoriteModelId: string) => {
    const next = toggledModelFavoriteIds(effectiveFavoriteIds, favoriteModelId);
    if (favoriteIds !== undefined && onFavoriteIdsChange) {
      setOptimisticFavoriteIds(next);
      void onFavoriteIdsChange(next).catch(error => {
        setOptimisticFavoriteIds(null);
        showError(error);
      });
      return;
    }
    updatePreferences({...preferences, favorites: next});
  }, [effectiveFavoriteIds, favoriteIds, onFavoriteIdsChange, preferences, updatePreferences]);
  const toggleGroup = useCallback((title: string) => {
    const next = new Set(preferences.collapsedGroups);
    if (next.has(title)) next.delete(title);
    else next.add(title);
    updatePreferences({...preferences, collapsedGroups: [...next]});
  }, [preferences, updatePreferences]);
  return (
    <PickerSheetFrame
      visible={visible}
      title="Choose model"
      subtitle={`${models.length} available from your desktop`}
      onClose={onClose}>
      <View style={styles.pickerSearch}>
        <MilimIcon name="search" size={15} color={palette.muted} />
        <TextInput
          style={styles.pickerSearchInput}
          value={query}
          onChangeText={setQuery}
          placeholder="Search models or providers"
          accessibilityLabel="Search models"
          placeholderTextColor={palette.placeholder}
          autoCapitalize="none"
          autoCorrect={false}
        />
        {query ? (
          <MotionPressable style={styles.pickerSearchClear} onPress={() => setQuery('')} hitSlop={8} accessibilityLabel="Clear search">
            <MilimIcon name="x" size={13} color={palette.muted} />
          </MotionPressable>
        ) : null}
      </View>
      <FlatList
        style={styles.pickerListView}
        contentContainerStyle={styles.pickerList}
        data={rows}
        keyExtractor={item => item.key}
        keyboardShouldPersistTaps="handled"
        initialNumToRender={10}
        windowSize={7}
        ListEmptyComponent={<Empty title="No matching models" copy="Try a model name, provider, or runtime." />}
        renderItem={({item}) => item.type === 'header' ? (
          <MotionPressable
            style={styles.pickerGroupHeader}
            disabled={!item.collapsible}
            accessibilityRole={item.collapsible ? 'button' : undefined}
            accessibilityState={item.collapsible ? {expanded: !item.collapsed} : undefined}
            onPress={() => item.collapsible && toggleGroup(item.title)}>
            {item.title === 'Favorites'
              ? <MilimIcon name="star" filled size={13} color={palette.accent} />
              : <ProviderIcon brand={item.brand} size={13} color={palette.secondary} />}
            <Text style={styles.pickerGroupTitle}>{item.title.toUpperCase()}</Text>
            <Text style={styles.pickerGroupCount}>{item.count}</Text>
            {item.collapsible
              ? <MilimIcon name={item.collapsed ? 'chevron-right' : 'chevron-down'} size={12} color={palette.muted} />
              : null}
          </MotionPressable>
        ) : item.type === 'effort' ? (
          <ReasoningEffortChoices
            model={item.model}
            selected={reasoningEffortOverrides?.[item.model.id] ?? 'auto'}
            onSelect={effort => onSelect(item.model.id, effort)}
          />
        ) : (
          <ModelPickerRow
            model={item.model}
            selected={item.model.id === selectedId}
            favorite={effectiveFavoriteIds.includes(item.model.id)}
            reasoningEffort={reasoningEffortOverrides?.[item.model.id] ?? 'auto'}
            onPress={() => onSelect(item.model.id)}
            onFavorite={() => toggleFavorite(item.model.id)}
            onToggleReasoning={item.model.reasoningEfforts.length
              ? () => setEffortModelId(current => current === item.model.id ? null : item.model.id)
              : undefined}
          />
        )}
      />
      <MotionPressable
        style={styles.pickerFavoritesOnly}
        accessibilityRole="switch"
        accessibilityState={{checked: preferences.favoritesOnly}}
        onPress={() => updatePreferences({...preferences, favoritesOnly: !preferences.favoritesOnly})}>
        <MilimIcon name="star" filled={preferences.favoritesOnly} size={14} color={preferences.favoritesOnly ? palette.accent : palette.muted} />
        <Text style={styles.pickerFavoritesOnlyText}>Favorites only</Text>
        <View style={[styles.pickerSwitch, preferences.favoritesOnly && styles.pickerSwitchOn]}>
          <View style={[styles.pickerSwitchThumb, preferences.favoritesOnly && styles.pickerSwitchThumbOn]} />
        </View>
      </MotionPressable>
    </PickerSheetFrame>
  );
}

export function ModelPickerRow({
  model,
  selected,
  favorite,
  reasoningEffort,
  onPress,
  onFavorite,
  onToggleReasoning,
}: {
  model: MobileModelOption;
  selected: boolean;
  favorite: boolean;
  reasoningEffort: string;
  onPress: () => void;
  onFavorite: () => void;
  onToggleReasoning?: () => void;
}) {
  const {palette, styles} = useAppTheme();
  const visibleCapabilities = onToggleReasoning
    ? model.capabilities.filter(capability => capability !== 'reasoning')
    : model.capabilities;
  return (
    <View style={[styles.pickerRow, selected && styles.pickerRowSelected]}>
      <MotionPressable style={styles.pickerRowMain} onPress={onPress} accessibilityState={{selected}}>
        <View style={styles.pickerRowIcon}>
          <ProviderIcon brand={model.brand} size={17} color={selected ? palette.accent : palette.secondary} />
        </View>
        <View style={styles.pickerRowBody}>
          <View style={styles.pickerRowTopline}>
            <Text style={styles.pickerRowTitle} numberOfLines={1}>{model.label}</Text>
            {selected ? <MilimIcon name="check" size={14} color={palette.accent} /> : null}
          </View>
          <View style={styles.pickerRowMeta}>
            <Text style={styles.pickerRowRoute} numberOfLines={1}>
              {[model.route, model.detail].filter(Boolean).join(' · ')}
            </Text>
            {visibleCapabilities.length ? (
              <View style={styles.capabilityRow}>
                {visibleCapabilities.slice(0, 5).map(capability => (
                  <View
                    key={capability}
                    style={[styles.pickerCapabilityIcon, capability === 'fast' && styles.pickerCapabilityFastIcon]}
                    accessibilityLabel={capability}>
                    <MilimIcon name={capabilityIcons[capability]} size={12} color={palette.muted} />
                  </View>
                ))}
              </View>
            ) : null}
          </View>
        </View>
      </MotionPressable>
      {onToggleReasoning ? (
        <MotionPressable
          style={styles.pickerRowEffort}
          hitSlop={4}
          accessibilityLabel={`Reasoning effort for ${model.label}: ${reasoningEffortLabels[reasoningEffort] ?? reasoningEffort}`}
          onPress={onToggleReasoning}>
          <MilimIcon name="sparkles" size={12} color={reasoningEffort === 'auto' ? palette.muted : palette.accent} />
          {reasoningEffort !== 'auto'
            ? <Text style={styles.pickerRowEffortText}>{reasoningEffortLabels[reasoningEffort] ?? reasoningEffort}</Text>
            : null}
        </MotionPressable>
      ) : null}
      <MotionPressable
        style={styles.pickerRowFavorite}
        hitSlop={4}
        accessibilityLabel={favorite ? `Remove ${model.label} from favorites` : `Add ${model.label} to favorites`}
        onPress={onFavorite}>
        <MilimIcon name="star" filled={favorite} size={14} color={favorite ? palette.accent : palette.muted} />
      </MotionPressable>
    </View>
  );
}

export function ReasoningEffortChoices({
  model,
  selected,
  onSelect,
}: {
  model: MobileModelOption;
  selected: string;
  onSelect: (effort: string) => void;
}) {
  const {styles} = useAppTheme();
  const choices = ['auto', ...new Set(model.reasoningEfforts)];
  return (
    <View style={styles.reasoningEffortChoices}>
      <Text style={styles.reasoningEffortLabel}>Reasoning</Text>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.reasoningEffortScroll}>
        {choices.map(effort => (
          <MotionPressable
            key={effort}
            style={[styles.reasoningEffortChoice, effort === selected && styles.reasoningEffortChoiceSelected]}
            onPress={() => onSelect(effort)}>
            <Text style={[styles.reasoningEffortChoiceText, effort === selected && styles.reasoningEffortChoiceTextSelected]}>
              {reasoningEffortLabels[effort] ?? effort}
            </Text>
          </MotionPressable>
        ))}
      </ScrollView>
    </View>
  );
}
