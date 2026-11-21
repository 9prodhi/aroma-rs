import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
run(items) { function accept(item) { return item.active; } return items.filter(accept); }
}
