// Actions for a project (spec 4.3). Enter opens the whole dev environment;
// its label follows the project's state: Focus when open, Locate when the folder is gone.
import { copyAction, pinAction, revealAction } from '@/actions/common'
import type { ActionProvider, RainAction, TargetOf } from '@/actions/types'
import { useLaunch } from '@/composables/useLaunch'
import type { Project } from '@/data/types'

const { runs, launch } = useLaunch()

// Enter and Ctrl+Enter act on the selection, so they must not start a second launch.
async function launchProject(project: Project, mode: 'full' | 'editorOnly') {
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
      id: 'terminal',
      label: 'Terminal here',
      icon: 'square-terminal',
      slot: 'more',
      // RainApi has no endpoint for this yet, so the prototype only reports it.
      run: ({ notice }) => notice.info(`Terminal opened in ${project.name}`),
    },
    {
      id: 'claude',
      label: 'New Claude session here',
      icon: 'bot',
      slot: 'more',
      // Starting sessions from Rain is out of scope (spec 10); the prototype only reports it.
      run: ({ notice }) => notice.info(`Claude session started in ${project.name}`),
    },
    revealAction(project.path, 'more'),
    copyAction('path', project.path),
  ]
}
