import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
sum(items) { let total = 0; for (const item of items) { total += item.amount; } return total; }
}
