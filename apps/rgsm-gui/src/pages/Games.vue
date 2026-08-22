<script lang="ts" setup>
import { computed, onMounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { Plus, Search, Star, Gamepad2 } from '@lucide/vue';
import { $t } from '../i18n';
import { error } from '../utils/logger';
import {
  commands,
  type FavoriteTreeNode,
  type Game,
} from '../api/commands';
import { getGameManagementPath } from '../composables/useGameManagementRoute';
import { useAddGameDrawer } from '../composables/useAddGameDrawer';
import { useConfig } from '../composables/useConfig';
import { useSaveListSort } from '../composables/useSaveListSort';
import KButton from '../ui/kit/KButton.vue';
import KInput from '../ui/kit/KInput.vue';
import KSegmented from '../ui/kit/KSegmented.vue';
import { collectLeafNames } from '../components/favoriteTreeContext';

const router = useRouter();
const { config, isGameVisible, saveConfig } = useConfig();
const { sortedGames } = useSaveListSort();
const { open: openAddGame } = useAddGameDrawer();

const searchQuery = ref('');
const viewMode = ref<'favorites' | 'all'>('favorites');
let viewInitialized = false;

// —— 游戏数据 ——
const games = computed(() =>
  sortedGames(config.value.games.filter((game) => isGameVisible(game.storage_key, game.name)))
);

const favoriteNames = computed(() => collectLeafNames(config.value?.favorites));

const visibleGames = computed(() => {
  const query = searchQuery.value.trim().toLowerCase();
  const base = viewMode.value === 'favorites'
    ? games.value.filter((g) => favoriteNames.value.has(g.name))
    : games.value;
  if (!query) return base;
  return base.filter((game) => game.name.toLowerCase().includes(query));
});

// —— 自动备份状态点 ——
const autoBackupGames = ref<Set<string>>(new Set());
async function refreshAutoBackup() {
  try {
    const result = await commands.getAutoBackupStatus();
    if (result.status === 'ok') {
      autoBackupGames.value = new Set(result.data.map((row) => row.game_name));
    }
  } catch (e) {
    error(`refresh auto-backup status error: ${e}`);
  }
}
onMounted(refreshAutoBackup);
watch(
  () => [
    (config.value?.games ?? []).map((g) => `${g.name}:${g.auto_backup ? 1 : 0}`).join('|'),
    JSON.stringify(config.value?.quick_action?.game_automations ?? []),
  ],
  refreshAutoBackup
);

// —— 视图初始化（无收藏默认「全部」）——
watch(
  () => config.value,
  (cfg) => {
    if (viewInitialized || !cfg) return;
    if (cfg.games.length === 0 && collectLeafNames(cfg.favorites).size === 0) return;
    viewInitialized = true;
    if (collectLeafNames(cfg.favorites).size === 0) viewMode.value = 'all';
  },
  { immediate: true }
);

// —— 收藏切换 ——
function removeFavoriteLeaf(nodes: FavoriteTreeNode[], name: string): boolean {
  const index = nodes.findIndex((node) => node.is_leaf && node.label === name);
  if (index >= 0) {
    nodes.splice(index, 1);
    return true;
  }
  for (const node of nodes) {
    if (!node.is_leaf && node.children && removeFavoriteLeaf(node.children, name)) return true;
  }
  return false;
}

async function toggleFavorite(game: Game) {
  if (!config.value) return;
  const favorites = [...(config.value.favorites ?? [])];
  if (favoriteNames.value.has(game.name)) {
    removeFavoriteLeaf(favorites, game.name);
  } else {
    favorites.push({
      label: game.name,
      is_leaf: true,
      children: null,
      node_id: crypto.randomUUID(),
    });
  }
  config.value.favorites = favorites;
  await saveConfig();
}

// —— 首字母渐变封面 ——
const GRADIENTS = [
  ['#6366f1', '#8b5cf6'],
  ['#0ea5e9', '#06b6d4'],
  ['#f43f5e', '#fb923c'],
  ['#10b981', '#34d399'],
  ['#f59e0b', '#ef4444'],
  ['#8b5cf6', '#ec4899'],
  ['#3b82f6', '#8b5cf6'],
  ['#14b8a6', '#3b82f6'],
];

function coverOf(game: Game): { char: string; gradient: string[] } {
  const name = game.name.trim();
  const char = (name.charAt(0) || '?').toUpperCase();
  let hash = 0;
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) >>> 0;
  return { char, gradient: GRADIENTS[hash % GRADIENTS.length] };
}

function goGame(game: Game) {
  router.push(getGameManagementPath(game.name));
}
</script>

<template>
  <div class="games-page">
    <!-- 顶部标题区 -->
    <header class="page-header">
      <div class="title-block">
        <h1 class="page-title">{{ $t('games.title') }}</h1>
        <p v-if="visibleGames.length" class="page-subtitle">
          {{ $t('games.count', { count: visibleGames.length }) }}
        </p>
      </div>
      <div class="toolbar">
        <KInput
          v-model="searchQuery"
          size="sm"
          class="search-input"
          :placeholder="$t('misc.search')"
        />
        <KSegmented
          v-model="viewMode"
          class="seg"
          :options="[
            { value: 'favorites', label: $t('misc.favorites') },
            { value: 'all', label: $t('sidebar.all_games') },
          ]"
        />
        <KButton size="sm" variant="primary" @click="openAddGame()">
          <template #icon><Plus :size="13" /></template>
          {{ $t('sidebar.add_game') }}
        </KButton>
      </div>
    </header>

    <!-- 卡片网格 -->
    <div v-if="visibleGames.length" class="games-grid">
      <button
        v-for="game in visibleGames"
        :key="game.name"
        type="button"
        class="game-card"
        @click="goGame(game)"
      >
        <div
          class="game-cover"
          :style="{
            background: `linear-gradient(135deg, ${coverOf(game).gradient[0]}, ${coverOf(game).gradient[1]})`,
          }"
        >
          <span class="cover-char">{{ coverOf(game).char }}</span>
          <span
            v-if="autoBackupGames.has(game.name)"
            class="cover-badge"
            :title="$t('sidebar.auto_backup_on')"
          />
        </div>
        <div class="game-body">
          <span class="game-name">{{ game.name }}</span>
          <button
            type="button"
            class="fav-btn"
            :class="{ faved: favoriteNames.has(game.name) }"
            :aria-label="
              favoriteNames.has(game.name)
                ? $t('favorite.remove')
                : $t('favorite.add_to_favorite')
            "
            @click.stop="toggleFavorite(game)"
          >
            <Star
              :size="15"
              :fill="favoriteNames.has(game.name) ? 'currentColor' : 'none'"
            />
          </button>
        </div>
      </button>
    </div>

    <!-- 空态 -->
    <div v-else class="empty-state">
      <Gamepad2 :size="40" class="empty-icon" />
      <p v-if="searchQuery.trim()" class="empty-text">{{ $t('misc.no_search_results') }}</p>
      <template v-else>
        <p class="empty-text">{{ $t('games.empty') }}</p>
        <KButton size="sm" variant="primary" @click="openAddGame()">
          <template #icon><Plus :size="13" /></template>
          {{ $t('sidebar.add_game') }}
        </KButton>
      </template>
    </div>
  </div>
</template>

<style scoped>
.games-page {
  display: flex;
  flex-direction: column;
  gap: 1.1rem;
  height: 100%;
  min-height: 0;
}

.page-header {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.75rem 1rem;
}

.title-block {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
}

.page-title {
  margin: 0;
  font-size: 1.25rem;
  font-weight: 650;
  color: var(--text);
}

.page-subtitle {
  margin: 0;
  font-size: 0.8rem;
  color: var(--text-dim);
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.search-input {
  width: 220px;
}

.games-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 0.9rem;
  overflow-y: auto;
  min-height: 0;
  padding: 2px 2px 8px;
}

.game-card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface);
  cursor: pointer;
  text-align: left;
  padding: 0;
  font: inherit;
  transition:
    transform 0.18s ease,
    border-color 0.18s ease,
    box-shadow 0.18s ease;
}

.game-card:hover {
  transform: translateY(-2px);
  border-color: var(--border-strong);
  box-shadow: var(--shadow-overlay);
}

.game-card:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.game-cover {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  height: 110px;
  flex-shrink: 0;
}

.cover-char {
  font-size: 2.4rem;
  font-weight: 700;
  color: rgba(255, 255, 255, 0.92);
  text-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
  user-select: none;
}

.cover-badge {
  position: absolute;
  top: 8px;
  right: 8px;
  width: 9px;
  height: 9px;
  border-radius: 50%;
  border: 2px solid rgba(255, 255, 255, 0.85);
  background: var(--success);
}

.game-body {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.55rem 0.6rem 0.55rem 0.7rem;
}

.game-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 0.85rem;
  color: var(--text);
}

.fav-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  padding: 3px;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-dim);
  opacity: 0;
  transition:
    opacity 0.15s ease,
    color 0.15s ease,
    background-color 0.15s ease;
}

.game-card:hover .fav-btn,
.fav-btn.faved {
  opacity: 1;
}

.fav-btn.faved {
  color: var(--text);
}

.fav-btn:hover {
  color: var(--text);
  background: var(--surface-2);
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.8rem;
  flex: 1;
  color: var(--text-dim);
}

.empty-icon {
  opacity: 0.6;
}

.empty-text {
  margin: 0;
  font-size: 0.9rem;
}
</style>
