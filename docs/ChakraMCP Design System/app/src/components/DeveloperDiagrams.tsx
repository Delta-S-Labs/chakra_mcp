import type { ReactNode } from 'react'

type FlowNode = {
  label: string
  title: string
  detail: string
}

type DeveloperFlowDiagramProps = {
  eyebrow: string
  title: string
  body: string
  tone?: 'butter' | 'coral' | 'ink'
  nodes: readonly FlowNode[]
  aside?: ReactNode
}

export function DeveloperFlowDiagram({
  eyebrow,
  title,
  body,
  tone = 'butter',
  nodes,
  aside,
}: DeveloperFlowDiagramProps) {
  return (
    <article className={`developer-diagram developer-diagram--${tone}`}>
      <header className="developer-diagram__header">
        <div className="eyebrow">{eyebrow}</div>
        <h3>{title}</h3>
        <p>{body}</p>
      </header>

      <div className="developer-diagram__sequence" role="list">
        {nodes.map((node, index) => (
          <div className="developer-diagram__segment" key={`${title}-${node.title}`}>
            <article className="developer-flow-node" role="listitem">
              <div className="developer-flow-node__label">{node.label}</div>
              <h4>{node.title}</h4>
              <p>{node.detail}</p>
            </article>
            {index < nodes.length - 1 ? (
              <div
                aria-hidden="true"
                className="developer-flow-node__connector"
              />
            ) : null}
          </div>
        ))}
      </div>

      {aside ? <div className="developer-diagram__aside">{aside}</div> : null}
    </article>
  )
}
