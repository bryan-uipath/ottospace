hi, i'd like to continue work on ottospace. Some scenarios I'm thinking through:

goal: the ultimate productivity agentic harness

0. Let's get a basic terminal split infrastructure working

1. Distraction-free mode - by subsuming the existing MacOS space, I could get control over notifications / badges. I'm easily distracted (maybe ADHD?) by slack message during my day. So I'd like to create a way to carve out focus blocks where Slack usage is enforced. Some additional thoughts:
  a. it'd be interesting if we could keep rescuetime-style metrics about which apps are used and where, so as part of my weekly review skill we could consider how to be more efficient.
  b. it'd be interesting to have an agent be able to audit the app (ie, slack via the cli or directly in the UI) and break through the do-not-disturb  / focus mode if a critical issue came up.

2. Fast application switching - I find the 'Command+Space' experience to be quite lacking - it takes very long to search recently used applications, like Outlook, Slack, etc.

3. The 'routines' ottobot could be subsumed by this tool.

4. Second- or third- screen layouts. I'm imagining the tool could handle my entire workspace across displays
  a. Display affinity - match layouts to certain monitors


5. Scripting / additional window handling
  a. As an extension to our devtool experiment, where we captured the devtool that popped open in Chrome - i wonder if we could generalize this. We could private a scripting language that handles new windows opening for 

6. Window persistence
  a. Apps like Supacode handle terminal persistence - could we extend that to app persistence?
  b. Maybe thin script wrappers around apps that can handle persistence. It wouldn't necessarily need to be _full_ persistence, but something like:
    1. tabs / urls open in chrome
    2. the current conversation or channel in slack
    3. the project open in vscode
    4. for terminals - cwd and past output

7. Window layout -
  a. I'm thinking a left bar that has top-level categories
    1. Applications -> quick access to applications that take up a full split (outlook, slack, etc)
    2. Workbook
      - Maestro DRI -> tab with dashboards, tab withc onfluence docs, etc

    3. Tasks
    4. Reposities
      - notes repositories
      - Show list of repositories and worktrees
  b. When opening the left bar, a secondary bar is opened
  c. There's a main canvas of splits
  d. The secondary bar can always be closed

8. Keyboard accessibility
  - For in-built windows, we could use the accessibility tools (prototyped in rust-ax-walker, but likely could be much improved and much faster)
  - We could ues a 'Control+G' sneak mode to make everything accessible
  - Should be a hierarchy of 
  - Windows can be created iterm style Command+Shift+D

9. Status bar
  - Show keyboard mode - are we in a vim style mode?

10. Tasks
  - Generally, i spend most of my day bouncing around LLM conversations
  - A task is a combination of:
    - a working directory
    - one or  multiple llm conversations
    - a priority
    - different resources (repositories needed, or worktrees)
    - when completed, should close and clean up
    - potentially associated with a project
  - My active conversations should be tracked as tasks
    - can be ordered by which ones need input / a next action in order of priority
    - should be presented to me in a clear view: ASD-STE100 text, ideally with architecture diagrams or visual evidence (before/after gifs)
  - these tasks are essentially mini projects

hierarchy should be similar to other apps like supacode:
a pane -> tabs -> window splits


Bonus features - some things that are currently awkward that this could solve:
1. Using finder is awkward for simple file exploration - wonder if we could have a keyboard accessible file explorer (vim style keys) that could be opened in splits
2. Opening preview gifs is awkward - maybe we could 
3. Could be extendi to worktrees
4. Markdown preview (vim in terminal is fine for editing)
5. Diff view / PR reviews



Interesting scenarios:
1. A 'dri' tab that has various dashboards and onboarding doc
2. Agentic interaction with the ax tools - writing draft messages for me directly in slack, for me to edit and approve
3. a project folder where the dev server and browser pointing to it are preloaded
4. instead of searching worktree-by-worktree for next action, i can just pop the next proirity task that needs my attention
