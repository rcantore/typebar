# Delegación nocturna: estado

Experimento arrancado el 2026-10-02. Un issue por noche delegado a una sesión de Claude Code en la nube (claude.ai/code), PR a la mañana. Las primeras dos semanas la tabla se lleva a mano acá. Los issues de GitHub son solo para los agentes; el tablero real vive en Focus.

## Tabla

| Issue | Fecha run | Modelo | Cupo aprox | Min revisión | Resultado | Notas |
|-------|-----------|--------|------------|--------------|-----------|-------|
| #14 | 2026-10-03 02:00 | sonnet | ~60k tokens (estimación del agente) | sin timer | merge limpio | setext headings (smoke test). PR #15, 1 commit, mergeado 2026-10-04 08:49. Corrida: 86 s, 11 turnos. |
| #16 | 2026-10-05 02:00 (armada) | sonnet | | | | render: parsear una vez por frame (paso 1 del plan de perf). Línea base main 3002dee: ARCHITECTURE 22.4 ms/frame, README 10.7 ms. |

Resultado: `merge limpio` / `corregido` / `descartado`. Cupo: lo que reporte la sesión en el comentario de cierre, o estimación a ojo desde el uso del plan. Min revisión: el timer de Focus `review #N`.

Notas de la corrida #14 (2026-10-03):

- `gh` no está autenticado en el sandbox de la nube, pero el entorno trae un MCP de GitHub y la sesión lo usó sola para abrir el PR y comentar el issue. El fallback del prompt no hizo falta.
- La sesión queda viva después de terminar, suscripta al PR: se despertó sola cuando pasó CI y cuando se mergeó. Si se le escribe desde la web, responde, pero no tiene conectores (no puede tocar Focus a propósito).
- El squash merge desde GitHub agrega `Co-authored-by: Claude` al commit de main porque el autor del commit del PR es la sesión. Si molesta, se borra en el diálogo de squash.
- El agente no verificó que los tests nuevos fallaran en `main` sin el fix (lo dejó anotado en el PR). Para el próximo issue, pedirlo explícito en el prompt.

## Protocolo por issue

1. El issue existe en GitHub con causa, comportamiento esperado, criterio de aceptación, caso de prueba y sección de entrega. Crear el issue es despachar: no hay issues en borrador.
2. La tarea espejo en Focus lleva el número de issue en el nombre (`#N ...`).
3. Armar la routine para esa noche: routine "typebar: issue nocturno (a demanda)", id `trig_01BwXNzvw7p8zLEvauowWdc9`, modelo Sonnet, sin conectores, prompt v3. Se arma a mano cada vez cambiándole `run_once_at` (02:00 Buenos Aires = 05:00 UTC); dispara una vez y queda desarmada. No es un cron diario. El prompt elige solo el issue: el abierto de número más bajo, autor rcantore, sin PR en rama `issue-N-*`.
4. A la mañana: timer `review #N` sobre la tarea de Focus, revisar el diff, mergear o rechazar. Si algo falló, `list_runs` + `get_run_log` de la routine dan el log de la corrida.
5. Anotar la fila en la tabla. Convención: el número de issue viaja en rama (`issue-N-...`), PR, comentario de cierre y descripción del timer.
6. Horario: los días hábiles de 09:00 a 15:00 (Buenos Aires) son hora pico y el cupo de la ventana de 5 horas se gasta más rápido. Las corridas nocturnas quedan fuera de ese rango a propósito.

## Prompt de la routine (v3, 2026-10-04, selector automático)

Cambios sobre v2: línea base de benchmark en main antes de tocar; tests nuevos verificados en rojo sobre main cuando es un bug; commit con la identidad de Roberto para que el squash no agregue Co-authored-by; fallback a las herramientas MCP de GitHub del sandbox (gh no está autenticado).

```
Sos una sesión nocturna autónoma sobre el repo rcantore/typebar (editor Markdown para terminal, en Rust; workspace con los crates typebar-core y typebar-tui). Nadie va a responder preguntas: ante una duda, elegí la opción más simple y dejala escrita en el PR.

1. Elegir el issue. Listá los issues abiertos con `curl -s "https://api.github.com/repos/rcantore/typebar/issues?state=open&per_page=50"` (el repo es público, no hace falta auth) y descartá las entradas que tengan la clave `pull_request` (son PRs). Candidatos: autor `rcantore` (campo user.login) y sin PR ya abierto o mergeado cuya rama (campo head.ref en `https://api.github.com/repos/rcantore/typebar/pulls?state=all&per_page=50`) empiece con `issue-N-`, siendo N el número del issue. Tomá el candidato de número más bajo. Si no hay ninguno, terminá con el resumen "Sin issues pendientes" y no toques nada.

2. Leé el issue elegido completo (cuerpo y comentarios). Trae causa, comportamiento esperado, criterio de aceptación, caso de prueba sugerido y sección de entrega. Si no tiene criterio de aceptación, no lo hagas: terminá explicando por qué.

3. Línea base antes de tocar nada, todavía parado en `main`: si el issue pide benchmark, corré los comandos de benchmark del issue y guardá la salida completa para el PR. Si el issue es un bug con tests nuevos, escribí primero los tests y verificá que fallan en `main` sin el fix; decilo en el PR.

4. Creá la rama desde `main`. Si el issue indica el nombre de rama, usá ese; si no, `issue-N-<slug-corto>`.

5. Implementá el fix siguiendo el issue, con los tests que pide (ajustalos si un helper no encaja, sin bajar la cobertura). No toques nada fuera del alcance del issue: ni CI, ni Cargo.toml, ni dependencias, ni comportamiento que el issue no mencione.

6. Verificá que pasen los tres, en este orden: `cargo test --workspace`, `cargo fmt --check` (si falla, corré `cargo fmt` y repetí), `cargo clippy --all-targets -- -D warnings`. Si el issue pide benchmark, corré los mismos comandos en la rama y comparalos con la línea base contra la condición del issue. Si alguno sigue fallando después de intentar arreglarlo, no abras PR: pusheá la rama igual y explicá qué falló en el resumen.

7. Commit con el estilo del repo: `fix(tui): ...`, `feat(core): ...`, `perf(tui): ...`, etc., en español y sin acentos en el subject, como los commits existentes. Sin trailer Co-Authored-By. Usá esta identidad de autor para que el squash merge no agregue co-autores: `git -c user.name="Roberto Cantore Galvez" -c user.email="rcantore@users.noreply.github.com" commit -m "..."`.

8. Pusheá la rama y abrí un PR contra `main` con título `<tipo>(<scope>): <resumen> (#N)` y cuerpo con: resumen del cambio, salida resumida de los tres comandos, las salidas de benchmark antes y después si el issue las pide, supuestos tomados, y la línea `Closes #N`. Usá `gh pr create` si está disponible y autenticado; si no, usá las herramientas MCP de GitHub del entorno; si tampoco están, dejá el título y el cuerpo del PR en el resumen final para que se cree a mano.

9. Comentá en el issue con el enlace al PR y una estimación de tokens o cupo consumidos (`gh issue comment N` o la herramienta MCP de GitHub). Si no hay ninguna de las dos, incluilo en el resumen final.

Terminá siempre con un resumen: issue elegido, rama, PR (o por qué no), resultado de los tres comandos, benchmark antes y después si aplica, supuestos, y la estimación de consumo.
```
