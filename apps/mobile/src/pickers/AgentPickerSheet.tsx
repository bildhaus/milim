import {useEffect, useMemo, useState} from 'react';
import {FlatList, TextInput, View} from 'react-native';
import {useMilimController} from '../controller/useMilimController';
import {MilimIcon} from '../ui/MilimIcon';
import {AgentAvatar} from '../ui/AgentAvatar';
import {useAppTheme, Text} from '../ui/appTheme';
import {MotionPressable} from '../ui/motion';
import {Empty} from '../ui/controls';
import {PickerSheetFrame} from '../ui/PickerSheetFrame';

export function AgentPickerSheet({
  visible,
  agents,
  selectedId,
  onClose,
  onSelect,
}: {
  visible: boolean;
  agents: NonNullable<ReturnType<typeof useMilimController>['bootstrap']>['agents'];
  selectedId: string | null;
  onClose: () => void;
  onSelect: (id: string) => void;
}) {
  const {palette, styles} = useAppTheme();
  const [query, setQuery] = useState('');
  const filtered = useMemo(() => agents.filter(agent =>
    !query.trim() || [agent.name, agent.description, agent.id].some(value =>
      value.toLowerCase().includes(query.trim().toLowerCase()),
    ),
  ), [agents, query]);
  useEffect(() => {
    if (!visible) setQuery('');
  }, [visible]);
  return (
    <PickerSheetFrame
      visible={visible}
      title="Choose Agent"
      subtitle="Use the same Agent definitions as milim desktop"
      onClose={onClose}>
      <View style={styles.pickerSearch}>
        <MilimIcon name="search" size={15} color={palette.muted} />
        <TextInput
          style={styles.pickerSearchInput}
          value={query}
          onChangeText={setQuery}
          placeholder="Search Agents"
          accessibilityLabel="Search Agents"
          placeholderTextColor={palette.placeholder}
        />
      </View>
      <FlatList
        data={filtered}
        keyExtractor={agent => agent.id}
        contentContainerStyle={styles.pickerList}
        keyboardShouldPersistTaps="handled"
        initialNumToRender={12}
        windowSize={7}
        ListHeaderComponent={!query ? (
          <MotionPressable style={[styles.pickerRow, !selectedId && styles.pickerRowSelected]} onPress={() => onSelect('')} accessibilityState={{selected: !selectedId}}>
            <View style={styles.pickerRowIcon}>
              <MilimIcon name="x" size={14} color={palette.secondary} />
            </View>
            <View style={styles.pickerRowBody}>
              <Text style={styles.pickerRowTitle}>No Agent</Text>
              <Text style={styles.pickerRowRoute}>Use the thread’s regular model and tools</Text>
            </View>
            {!selectedId ? <MilimIcon name="check" size={15} color={palette.accent} /> : null}
          </MotionPressable>
        ) : null}
        ListEmptyComponent={<Empty title="No matching Agents" copy="Agent names and descriptions are searchable here." />}
        renderItem={({item: agent}) => (
          <MotionPressable
            style={[styles.pickerRow, agent.id === selectedId && styles.pickerRowSelected]}
            accessibilityState={{selected: agent.id === selectedId}}
            onPress={() => onSelect(agent.id)}>
            <View style={[styles.pickerRowIcon, agent.id === selectedId && styles.pickerRowIconSelected]}>
              <AgentAvatar {...agent} size={24} />
            </View>
            <View style={styles.pickerRowBody}>
              <View style={styles.pickerRowTopline}>
                <Text style={styles.pickerRowTitle} numberOfLines={1}>{agent.name}</Text>
                {agent.id === selectedId ? <MilimIcon name="check" size={15} color={palette.accent} /> : null}
              </View>
              <Text style={styles.pickerRowDescription} numberOfLines={2}>{agent.description || 'Custom milim Agent'}</Text>
              <Text style={styles.pickerRowRoute}>
                {agent.enabled_tool_count} tools · {agent.enabled_skill_count} skills
              </Text>
            </View>
          </MotionPressable>
        )}
      />
    </PickerSheetFrame>
  );
}
