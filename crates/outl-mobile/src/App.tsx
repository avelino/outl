import { Journal } from "./components/Journal";

function App() {
  return (
    <div class="flex h-full flex-col bg-(--color-ios-bg) dark:bg-(--color-iosd-bg)">
      <Journal />
    </div>
  );
}

export default App;
