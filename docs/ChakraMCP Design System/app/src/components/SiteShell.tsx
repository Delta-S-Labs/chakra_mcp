import { NavLink } from 'react-router-dom'
import type { PropsWithChildren } from 'react'

type SiteShellProps = PropsWithChildren<{
  kicker: string
}>

export function SiteShell({ children, kicker }: SiteShellProps) {
  return (
    <div className="site-shell">
      <header className="site-header">
        <div className="brand-lockup">
          <div className="brand-mark">Agent Telepathy</div>
          <div className="brand-kicker">{kicker}</div>
        </div>
        <nav aria-label="Primary" className="site-nav">
          <NavLink
            className={({ isActive }) =>
              isActive ? 'nav-link active' : 'nav-link'
            }
            to="/"
          >
            Portfolio
          </NavLink>
          <NavLink
            className={({ isActive }) =>
              isActive ? 'nav-link active' : 'nav-link'
            }
            to="/concept"
          >
            Concept
          </NavLink>
          <NavLink
            className={({ isActive }) =>
              isActive ? 'nav-link active' : 'nav-link'
            }
            to="/developer"
          >
            Developer
          </NavLink>
        </nav>
      </header>
      <main className="site-main">{children}</main>
      <footer className="site-footer">
        <div className="footer-note">
          A relay-first MCP network for agents with public menus, private
          friendships, and no patience for sloppy permissions.
        </div>
      </footer>
    </div>
  )
}
