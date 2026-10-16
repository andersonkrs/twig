

# twig

Un gestor de sesiones de tmux con soporte para git worktree, inspirado en [tmuxinator](https://github.com/tmuxinator/tmuxinator).

Creado para satisfacer mi propia necesidad. Interfaz de terminal construida con [Ratatui](https://ratatui.rs/).

Cuando estás equilibrando funciones, correcciones y revisiones, [git worktree](https://git-scm.com/docs/git-worktree) te permite mantener varias ramas activadas simultáneamente. Twig vincula cada worktree a una sesión de tmux y proporciona una TUI ágil para que puedas iniciar un espacio de trabajo limpio y enfocado por rama en segundos.


https://github.com/user-attachments/assets/95e21d30-a055-4c1b-b0b6-dcccb61dd53e

¿Necesitas revisar el PR de un compañero? En la vista de árbol (`twig tree list`), ingresa `#123` como nombre de rama y twig usará `gh` para obtener la cabeza del PR (incluyendo forks), creará una rama local `pr-123` e iniciará un worktree. Requiere autenticación de GitHub CLI (`gh`).


## Requisitos

- `tmux`
- `git`

## Instalación

Recomendamos usar [mise](https://mise.jdx.dev/) para la instalación.

### Vía mise

```bash
mise use -g cargo:https://github.com/andersonkrs/twig
```

Esto compila twig desde el código fuente y lo instala de forma global.

### Desde el código fuente (para desarrollo)

```bash
git clone https://github.com/andersonkrs/twig.git ~/Work/twig
cd ~/Work/twig

# Install all tools (rust, lefthook) + git hooks
mise install

# Build
cargo build --release

# Symlink to PATH
ln -s ~/Work/twig/target/release/twig ~/.local/bin/twig
```

## Uso

```bash
twig start [project]     # Start/attach to session (interactive if no arg)
twig list                # List all projects/worktrees
twig list --focus-current # Focus current TWIG_PROJECT/TWIG_WORKTREE
twig new [name|repo_url] # Create new project (accepts name or git URL)
twig edit [project]      # Open config in $EDITOR
twig delete [project]    # Delete project config
twig stop [project]      # Kill tmux session

# Debug tmux control-mode I/O
Use `--verbose` (or `TWIG_DEBUG=1`) to enable verbose tmux control output on stderr.
twig --verbose window new [project] [name]

# Run a command in a window/pane
twig run --project=dotfiles --window=6 --pane=1 -- whoami

# Run a command in a worktree session
twig run --project=dotfiles --tree=feature-x --window=1 -- btop

# Activate handoff windows for a target session/worktree
twig window activate --project=myproject
twig window activate --project=myproject --tree=feature-auth

# Worktree commands
twig tree create [project] [branch]   # Create worktree + session
twig tree list [project]              # List worktrees
twig tree delete [project] [branch]   # Delete worktree + kill session
```

Al crear un proyecto con una URL de git, twig extrae automáticamente el nombre del proyecto:
```bash
twig new git@github.com:user/myproject.git  # Creates project "myproject"
```

Alias: `ls` para `list`, `s` para `start`, `n` para `new`, `e` para `edit`, `rm` para `delete`, `t` para `tree`

## Configuración

### Configuración Global

Ubicación: `~/.config/twig/config.yml`

```yaml
# Base path for worktrees (default: ~/Work/.trees)
# Worktrees are created at: {worktree_base}/{project}/{branch}
worktree_base: ~/Work/.trees

# Projects directory (default: ~/.config/twig/projects)
projects_dir: ~/.config/twig/projects
```

### Configuración del Proyecto

Ubicación: `~/.config/twig/projects/<name>.yml`

```yaml
name: myproject
root: ~/Work/myproject

# Optional: git repo URL (https or ssh)
# If root doesn't exist, twig will clone this repo on first start
repo: git@github.com:user/myproject.git

windows:
  # Simple window with command
  - git: lazygit

  # Empty shell window
  - shell:

  # Window with multiple panes
  - editor:
      panes:
        - nvim

  # Window with layout and multiple panes
  - servers:
      layout: main-vertical    # main-vertical, main-horizontal, even-vertical, even-horizontal, tiled
      panes:
        - rails server
        - bin/sidekiq

# Optional: worktree configuration
worktree:
  # Files/folders to copy from parent project to worktree
  copy:
    - .env
    - .env.local
    - config/master.key

  # Files/folders to symlink from parent project to worktree
  # Only supported on Unix
  symlink:
    - .env

  # Commands to run after worktree creation
  post_create:
    - bundle install
    - yarn install
    - rails db:migrate

  # Optional: windows managed by handoff activation.
  # Run `twig window activate` to pause these windows in other sessions
  # for this project and start them in the target session.
  handoff_windows:
    - rails

  # Note: post_create runs inside a temporary setup window in the worktree session
  # so your shell init and environment (mise/rbenv/etc) are applied.
```

### Ejemplos de Configuración

**Proyecto Rails:**
```yaml
name: myapp
root: ~/Work/myapp

windows:
  - editor:
      panes:
        - nvim
  - shell:
  - rails:
      layout: main-vertical
      panes:
        - rails server
        - bin/sidekiq
  - console: rails console
  - git: lazygit

worktree:
  copy:
    - .env
    - .env.local
    - config/master.key
    - config/credentials.yml.enc
  symlink:
    - .env
  post_create:
    - bundle install
    - yarn install
    - bin/rails db:prepare

  # Optional: windows managed by handoff activation.
  # Run `twig window activate` to pause these windows in other sessions
  # for this project and start them in the target session.
  handoff_windows:
    - rails
```

**Proyecto simple:**
```yaml
name: dotfiles
root: ~/.dotfiles

windows:
  - editor:
      panes:
        - nvim
  - shell:
  - shell:
  - git: lazygit
```

## Cómo Funciona

Twig es una capa fina en Rust que convierte configuraciones YAML en comandos de modo de control de tmux y gestiona los git worktrees cuando se solicita. La CLI orquesta la carga de configuración, la creación de worktrees de git y la construcción de sesiones de tmux; la TUI solo renderiza el estado y activa acciones de la CLI.

```text
YAML config
   |
   v
CLI (twig) ---> git worktree ops (optional)
   |
   v
tmux control mode -> tmux server -> sessions/windows/panes
```

```text
User input (TUI)
   |
   v
CLI commands -> tmux control mode
```

Documentación del protocolo de control de tmux: https://man7.org/linux/man-pages/man1/tmux.1.html#CONTROL_MODE

### Sesiones

Cuando ejecutas `twig start <project>`:

1. Verifica si la sesión ya existe → se conecta si es así
2. Crea una nueva sesión de tmux con las ventanas/paneles configurados
3. Ejecuta comandos en cada panel
4. Se conecta a la sesión (o cambia a ella si ya estás en tmux)

### Worktrees

Cuando ejecutas `twig tree create <project> <branch>`:

1. Crea el git worktree en `{worktree_base}/{project}/{branch}`
2. Si la rama no existe, obtiene el origen y la crea desde `origin/<default_branch>` para que siempre comience limpia
3. Copia y enlaza simbólicamente los archivos configurados desde el proyecto padre
4. Ejecuta los comandos post-creación
5. Inicia una sesión de tmux con el nombre `{project}__{branch}`

Nomenclatura de sesiones: `myproject__feature-auth` (separador con doble guión bajo)

Ruta del worktree: `~/Work/.trees/myproject/feature-auth`

### Activación de Handoff

`handoff_windows` solo se aplica cuando activas explícitamente una sesión objetivo.

Uso:

```bash
# Activate handoff for the main project session
twig window activate --project myproject

# Activate handoff for a worktree session
twig window activate --project myproject --tree feature-auth
```

Qué hace la activación:

1. Encuentra las ventanas listadas en `worktree.handoff_windows`
2. Envía señales de parada a las ventanas coincidentes en otras sesiones del proyecto
3. Inicia las ventanas coincidentes en la sesión objetivo activada

Consejos:

- Mantén los mismos nombres de ventana en las configuraciones principal/worktree (por ejemplo `rails`, `sidekiq`)
- Ejecuta la activación después de cambiar de contexto (principal -> worktree o worktree -> principal)
- En la vista de árbol (`twig tree list`), usa la acción Activar para hacerlo de forma interactiva

Cuando ejecutas `twig tree delete <project> <branch>`:

1. Finaliza la sesión de tmux si está en ejecución
2. Elimina el git worktree

## Selector de Sesiones en Popup de Tmux

Puedes reemplazar el selector de sesiones de tmux con un popup que ejecute `twig ls --focus-current`. Esto utiliza las variables de entorno `TWIG_PROJECT` y `TWIG_WORKTREE` para centrar el cursor en el proyecto/worktree actual cuando estén disponibles.

Agrega un atajo de teclado a tu `~/.tmux.conf`:

```tmux
# Twig popup
unbind s
bind-key s display-popup -E -w 80% -h 60% "twig ls --focus-current"
```

Si deseas que el popup se abra siempre desde cualquier lugar (no solo dentro de una sesión de twig), seguirá funcionando, pero volverá al primer proyecto cuando las variables de entorno no estén configuradas.

## Versiones (Releases)

- Las versiones son gestionadas por release-plz usando commits convencionales para determinar el incremento.
- Se crea/actualiza un PR de versión en cada push a `main`.
- Fusiona el PR de versión para etiquetar y publicar un release en GitHub; los binarios se suben para linux x86_64 y macOS universal2.
- Puedes volver a ejecutar las verificaciones de versión activando manualmente el flujo de trabajo "Release Plz".

## Desarrollo

```bash
# Install dependencies + git hooks
mise install

# Build
cargo build --release
```

El formato y el linting se ejecutan automáticamente con lefthook en el pre-commit.

## Licencia

MIT
