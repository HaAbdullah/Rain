// Actions for a project (spec 4.3). Enter opens the whole dev environment;
// its label follows the project's state: Focus when open, Locate when the folder is gone.
import { ref } from 'vue'
import { copyAction, pinAction, revealAction } from '@/actions/common'
import type { ActionContext, ActionProvider, RainAction, TargetOf } from '@/actions/types'
import { useLaunch } from '@/composables/useLaunch'
import type { Project } from '@/data/types'

const { runs, launch } = useLaunch()

/**
 * The layout editor's state. LayoutEditorDialog (mounted by ProjectsScreen) is open while `target`
 * is set; it saves with api.saveProjectLayout and sets it back to null.
 */
export const layoutEditor = {
  target: ref<{ id: string; name: string } | null>(null),

  edit(project: Project) {
    layoutEditor.target.value = { id: project.id, name: project.name }
  },
}

/** The editor dialog only exists on the Projects screen, so go there first. */
function editLayout(project: Project, ctx: ActionContext) {
  if (ctx.nav.displayed.value !== 'projects') ctx.nav.go('projects', undefined, { id: project.id })
  layoutEditor.edit(project)
}

// Enter and Ctrl+Enter act on the selection, so they must not start a second launch.
async function launchProject(project: Project, mode: 'full' | 'editorOnly' | 'terminal' | 'claude') {
  if (runs.value[project.id]?.status === 'running') return
  await launch({ type: 'project', id: project.id, mode }, project.name)
}

function primaryFor(project: Project): RainAction {
  if (project.missing) {
    return {
      id: 'locate',
      label: 'Locate',
      icon: 'settings',
      slot: 'primary',
      run({ nav, notice }) {
        nav.go('settings')
        notice.info(`${project.name}: folder not found. Check the project folders.`)
      },
    }
  }
  return {
    id: 'open',
    // Launching an open project brings its windows forward instead of starting them again.
    label: project.open ? 'Focus' : 'Open dev environment',
    icon: project.open ? 'focus' : 'rocket',
    slot: 'primary',
    run: () => launchProject(project, 'full'),
  }
}

export const projectActions: ActionProvider<TargetOf<'project'>> = ({ item: project }) => {
  const pin = pinAction('project', project.id, project.pinned)
  // Nothing can be opened in a folder that is not there.
  if (project.missing) return [primaryFor(project), pin, copyAction('path', project.path)]

  return [
    primaryFor(project),
    {
      id: 'editor',
      label: 'Open in editor only',
      icon: 'code-xml',
      slot: 'secondary',
      run: () => launchProject(project, 'editorOnly'),
    },
    pin,
    {
      id: 'layout',
      label: 'Edit layout',
      icon: 'layout-grid',
      slot: 'more',
      keys: 'Ctrl+KeyE',
      // Where each window goes when the project opens.
      run: (ctx) => editLayout(project, ctx),
    },
    {
      id: 'terminal',
      label: 'Terminal here',
      icon: 'square-terminal',
      slot: 'more',
      // One Windows Terminal tab in the project folder.
      run: () => launchProject(project, 'terminal'),
    },
    {
      id: 'claude',
      label: 'New Claude session here',
      icon: 'bot',
      slot: 'more',
      // One Windows Terminal tab in the project folder that runs Claude Code.
      run: () => launchProject(project, 'claude'),
    },
    revealAction(project.path, 'more'),
    copyAction('path', project.path),
  ]
}
