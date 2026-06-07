import { useCallback, useEffect, useMemo, useState } from "react";
import { loadAnimePosterUrls } from "../animePoster";
import type { AnimeSummary, Category, LibraryState } from "../types";
import { useRovingListNavigation } from "../useRovingListNavigation";
import { animeDisplayTitle } from "../utils";
import { ConfirmModal } from "./ConfirmModal";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "./ContextMenu";
import { PromptModal } from "./PromptModal";
import { ViewHeader } from "./ViewHeader";

export function CategoryScreen(props: {
  library: LibraryState;
  onOpenCategory: (categoryId: number) => void;
  onOpenAnime: (anime: AnimeSummary) => void;
  onOpenSettings: () => void;
  onCreateCategory: (name: string) => Promise<void>;
  onDeleteCategory: (category: Category) => void;
  onMoveCategoryToPosition: (category: Category, position: number) => void;
  onSetDefaultCategory: (category: Category) => void;
}) {
  const {
    library,
    onOpenCategory,
    onOpenAnime,
    onOpenSettings,
    onCreateCategory,
    onDeleteCategory,
    onMoveCategoryToPosition,
    onSetDefaultCategory,
  } = props;
  const getRovingItemProps = useRovingListNavigation(library.categories.length + library.recent_anime.length);
  const [recentCovers, setRecentCovers] = useState<Record<number, string>>({});
  const { menu, openMenu, closeMenu } = useContextMenu();
  const [addCategoryOpen, setAddCategoryOpen] = useState(false);
  const [addCategoryBusy, setAddCategoryBusy] = useState(false);
  const [addCategoryError, setAddCategoryError] = useState<string | null>(null);
  const [deleteCategory, setDeleteCategory] = useState<Category | null>(null);
  const animeByCategory = useMemo(() => {
    const counts = new Map<number, number>();
    for (const anime of library.anime) {
      counts.set(anime.category_id, (counts.get(anime.category_id) ?? 0) + 1);
    }
    return counts;
  }, [library.anime]);

  useEffect(() => {
    let cancelled = false;
    setRecentCovers({});
    void loadAnimePosterUrls(
      library.recent_anime,
      (animeId, url) => {
        setRecentCovers((current) => (cancelled ? current : { ...current, [animeId]: url }));
      },
      () => !cancelled,
    );
    return () => {
      cancelled = true;
    };
  }, [library.recent_anime]);

  const buildCategoryMenuItems = useCallback(
    (category: Category): ContextMenuItem[] => {
      const currentIndex = library.categories.findIndex((item) => item.id === category.id);
      const items: ContextMenuItem[] = [
        {
          type: "action",
          id: "add-category",
          label: "Add new category",
          onSelect: () => setAddCategoryOpen(true),
        },
        {
          type: "submenu",
          id: "move-category",
          label: "Move to position",
          items: library.categories.map((item, index) => ({
            id: `position-${index + 1}`,
            label: `${index + 1}. ${item.name}${item.id === category.id ? " (current)" : ""}`,
            disabled: index === currentIndex,
            disabledTitle: index === currentIndex ? "Already at this position" : undefined,
            onSelect: () => onMoveCategoryToPosition(category, index + 1),
          })),
        },
      ];

      if (!category.is_default) {
        items.push({
          type: "action",
          id: "set-default",
          label: "Set as default",
          onSelect: () => onSetDefaultCategory(category),
        });
      }

      items.push(
        { type: "separator", id: "delete-separator" },
        {
          type: "action",
          id: "delete-category",
          label: "Delete category",
          danger: true,
          disabled: category.is_default,
          disabledTitle: category.is_default ? "Cannot delete default category" : undefined,
          onSelect: () => setDeleteCategory(category),
        },
      );

      return items;
    },
    [library.categories, onMoveCategoryToPosition, onSetDefaultCategory],
  );

  const submitNewCategory = useCallback(
    async (name: string) => {
      const trimmed = name.trim();
      if (!trimmed) {
        setAddCategoryError("Category name is required.");
        return;
      }
      setAddCategoryBusy(true);
      setAddCategoryError(null);
      try {
        await onCreateCategory(trimmed);
        setAddCategoryOpen(false);
      } catch (error) {
        setAddCategoryError(error instanceof Error ? error.message : String(error));
      } finally {
        setAddCategoryBusy(false);
      }
    },
    [onCreateCategory],
  );

  return (
    <>
      <ViewHeader
        title="Library"
        subtitle="Browse videos by category, or continue where you left off."
      />

      {library.root_folders.length === 0 ? (
        <div className="empty empty--wide">
          <h2>Add a root folder to begin</h2>
          <p className="muted">The library scanner will group matching video filenames into titles and episodes.</p>
          <button type="button" onClick={onOpenSettings}>
            Open settings
          </button>
        </div>
      ) : null}

      <section className="category-grid">
        {library.categories.map((category, index) => {
          const count = animeByCategory.get(category.id) ?? 0;
          return (
            <button
              type="button"
              className="category-card"
              key={category.id}
              onClick={() => onOpenCategory(category.id)}
              onContextMenu={(event) => openMenu(event, buildCategoryMenuItems(category))}
              {...getRovingItemProps(index)}
            >
              <span className="category-name">{category.name}</span>
              <span className="category-count">
                {count} title{count === 1 ? "" : "s"}
              </span>
            </button>
          );
        })}
      </section>

      {library.recent_anime.length > 0 ? (
        <>
          <div className="section-heading">
            <h2>Continue Watching</h2>
          </div>
          <div className="continue-grid">
            {library.recent_anime.map((anime, index) => {
              const cover = recentCovers[anime.id];
              return (
                <button
                  type="button"
                  className={`continue-card${cover ? " continue-card--with-cover" : ""}`}
                  key={anime.id}
                  onClick={() => onOpenAnime(anime)}
                  {...getRovingItemProps(library.categories.length + index)}
                >
                  {cover ? (
                    <img className="continue-card-cover" src={cover} alt="" loading="lazy" />
                  ) : null}
                  <div className="continue-card-body">
                    <strong>{animeDisplayTitle(anime, library.prefer_anilist_display_title)}</strong>
                    <span>{anime.episode_count} episode{anime.episode_count === 1 ? "" : "s"}</span>
                  </div>
                </button>
              );
            })}
          </div>
        </>
      ) : null}

      <ContextMenu menu={menu} onClose={closeMenu} />

      {deleteCategory ? (
        <ConfirmModal
          title="Delete category?"
          description={`Delete "${deleteCategory.name}"? Titles in this category will move to the default category.`}
          onConfirm={() => {
            onDeleteCategory(deleteCategory);
            setDeleteCategory(null);
          }}
          onClose={() => setDeleteCategory(null)}
        />
      ) : null}

      {addCategoryOpen ? (
        <PromptModal
          title="Add category"
          description="Create a new library category."
          label="Category name"
          submitLabel="Create"
          busy={addCategoryBusy}
          error={addCategoryError}
          onSubmit={(value) => void submitNewCategory(value)}
          onClose={() => {
            if (addCategoryBusy) return;
            setAddCategoryOpen(false);
            setAddCategoryError(null);
          }}
        />
      ) : null}
    </>
  );
}
