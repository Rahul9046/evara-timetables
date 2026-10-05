import React from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

const days = ["Monday","Tuesday","Wednesday","Thursday","Friday"];
const lessons = [
  ["Mathematics · 7A","English · 8A","Science · 7A","History · 9A","Mathematics · 8A"],
  ["English · 7A","Science · 8A","Mathematics · 9A","English · 9A","Science · 9A"],
  ["Science · 9A","Mathematics · 7A","English · 7A","Science · 8A","History · 8A"]
];

function App(){
 return <main>
  <header><div><span className="eyebrow">OPEN SOURCE SCHOOL SCHEDULING</span><h1>Evara Timetables</h1><p>Build feasible school timetables around people, rooms, curriculum and real-world constraints.</p></div><button>Generate timetable</button></header>
  <section className="stats">
   <article><b>0</b><span>Hard violations</span></article><article><b>12</b><span>Soft penalties</span></article><article><b>15</b><span>Lessons scheduled</span></article><article><b>100%</b><span>Coverage</span></article>
  </section>
  <section className="panel"><div className="panelHead"><div><h2>Master timetable</h2><p>Demo data · Week A</p></div><span className="healthy">● Feasible</span></div>
   <div className="grid"><div className="corner">Period</div>{days.map(d=><div className="day" key={d}>{d}</div>)}
   {lessons.map((row,i)=><React.Fragment key={i}><div className="period">P{i+1}<small>{8+i}:30</small></div>{row.map((l,j)=><div className="lesson" key={j}><strong>{l}</strong><span>Room {101+j} · Teacher {j+1}</span></div>)}</React.Fragment>)}</div>
  </section>
  <section className="bottom"><article><h3>Constraint health</h3><p className="ok">✓ No hard constraints violated</p><p>Repeated subjects are spread across the week.</p><p>Teacher gaps can still be improved.</p></article><article><h3>What comes next</h3><p>Connect this interface to the optimization service, then make every lesson draggable with immediate constraint explanations.</p></article></section>
 </main>
}
createRoot(document.getElementById("root")!).render(<App/>);
